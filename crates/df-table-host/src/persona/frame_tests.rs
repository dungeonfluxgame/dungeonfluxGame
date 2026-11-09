//! The frame judge's settings and the checks on its answer; the watch's input, options and
//! schedule; the focus judgement and the frame timing lines.

use std::time::{Duration, Instant};

use serde_json::json;

use super::cli::{PersonaCommand, parse};
use super::converse::Speaker;
use super::frame::{
    FrameAsk, FrameConfig, FrameError, FrameMoment, FrameReason, FrameReply, FrameRequest,
    FrameScheduleRequest, FrameTimingReply, FrameTimingRequest, PERSONA_FRAME,
    PERSONA_FRAME_SCHEDULE, PERSONA_FRAME_TIMING, parse_frame, read_frames,
};
use super::frame_watch::{
    FrameWatch, MAXIMUM_PARALLEL_FRAMES, WatchInput, needs_lead, painted_prompt, picture_target,
    slot_due, slots_due,
};
use super::plate::{
    Painting, SharedScreen, claim_room, fit_pictures, plate_request, reference_sheets, with_sheets,
};
use super::story::StoryItem;
use crate::imagegen::limits::{CODEX_MAXIMUM_PICTURES, MAXIMUM_PICTURES};
use crate::imagegen::{BackendLimits, ThingReference};
use crate::locate::request::CharacterRequest;
use crate::movement::brief::BriefStyle;
use crate::world::PlaceItem;
use df_content::world_bible::Item;
use df_providers::{
    FORGE_IMAGE_MAXIMUM_DESCRIPTION_CHARACTERS, FORGE_IMAGE_MAXIMUM_NEGATIVE_CHARACTERS,
};

fn speakers() -> Vec<Speaker> {
    vec![Speaker {
        id: "tavern_server".to_owned(),
        display_name: "Lena".to_owned(),
        job: "Server".to_owned(),
    }]
}

#[test]
fn the_shipped_frame_file_is_the_built_in_settings() {
    let shipped: FrameConfig = serde_json::from_str(include_str!(
        "../../../../assets/playground/persona/frames.json"
    ))
    .unwrap();
    assert_eq!(shipped, FrameConfig::default());
    let missing = std::path::Path::new("no/such/frames.json");
    assert_eq!(read_frames(missing).unwrap(), FrameConfig::default());
}

#[test]
fn an_answer_is_kept_only_with_listed_characters_and_kinds_and_a_percent_to_100() {
    let config = FrameConfig::default();
    let answer = |character: serde_json::Value, kind: &str, percent: u64| {
        json!({
            "changed_percent": percent,
            "changes": [{"character": character, "kind": kind, "now": "smiling at Kael"}],
            "next_picture": "Lena smiles at Kael across the bar.",
            "shot": "close-up",
            "inspected_item": null,
            "new_look": false
        })
    };
    let parsed = parse_frame(
        &answer(json!("tavern_server"), "expression", 30),
        &speakers(),
        &[],
        &config,
    )
    .unwrap();
    assert_eq!((parsed.changed_percent, parsed.changes.len()), (30, 1));
    assert_eq!(parsed.next_picture, "Lena smiles at Kael across the bar.");
    assert_eq!(parsed.shot.id, "close-up");
    let mut wrong_shot = answer(json!(null), "light", 5);
    wrong_shot["shot"] = json!("selfie");
    assert_eq!(
        parse_frame(&wrong_shot, &speakers(), &[], &config).err(),
        Some(FrameError::UnknownShot {
            shot: "selfie".to_owned()
        })
    );
    assert!(parse_frame(&answer(json!(null), "light", 5), &speakers(), &[], &config).is_ok());
    assert_eq!(
        parse_frame(
            &answer(json!("rina"), "expression", 30),
            &speakers(),
            &[],
            &config
        ),
        Err(FrameError::UnknownCharacter {
            id: "rina".to_owned()
        })
    );
    assert_eq!(
        parse_frame(&answer(json!(null), "smell", 30), &speakers(), &[], &config),
        Err(FrameError::UnknownKind {
            kind: "smell".to_owned()
        })
    );
    assert!(matches!(
        parse_frame(
            &answer(json!(null), "light", 101),
            &speakers(),
            &[],
            &config
        ),
        Err(FrameError::MalformedAnswer { .. })
    ));
}

#[test]
fn an_inspected_item_is_one_offered_here_or_null() {
    let config = FrameConfig::default();
    let items = [PlaceItem {
        item: Item {
            id: "frostline_longsword".to_owned(),
            name: "Frostline longsword".to_owned(),
            unit: "each".to_owned(),
            base_value_cp: 45_000,
            tags: vec!["frost-runes".to_owned()],
        },
        keeper: "weapon_seller".to_owned(),
    }];
    let answer = |inspected: serde_json::Value| {
        json!({
            "changed_percent": 10,
            "changes": [],
            "next_picture": "The Frostline fills the frame, frost runes along the fuller.",
            "shot": "wide",
            "inspected_item": inspected,
            "new_look": true
        })
    };
    let parsed = parse_frame(
        &answer(json!("frostline_longsword")),
        &speakers(),
        &items,
        &config,
    )
    .unwrap();
    assert_eq!(
        parsed.inspected.map(|offered| offered.item.id).as_deref(),
        Some("frostline_longsword")
    );
    let nobody = parse_frame(&answer(json!(null)), &speakers(), &items, &config).unwrap();
    assert_eq!(nobody.inspected, None);
    assert_eq!(
        parse_frame(&answer(json!("teapot")), &speakers(), &items, &config),
        Err(FrameError::UnknownItem {
            id: "teapot".to_owned()
        })
    );
    assert_eq!(config.inspect_shot.as_deref(), Some("insert"));
    let mut chose_insert = answer(json!(null));
    chose_insert["shot"] = json!("insert");
    assert_eq!(
        parse_frame(&chose_insert, &speakers(), &items, &config),
        Err(FrameError::UnknownShot {
            shot: "insert".to_owned()
        }),
        "the insert is code's for an inspection, never the judge's choice"
    );
}

#[test]
fn a_request_asks_about_the_lines_unless_it_names_a_schedule_or_a_focus() {
    let request = |ask: serde_json::Value| {
        let mut body = json!({"picture": "A flooded scriptorium.", "lines": [], "speakers": []});
        if !ask.is_null() {
            body["ask"] = ask;
        }
        serde_json::from_value::<FrameRequest>(body)
    };
    assert_eq!(request(json!(null)).unwrap().ask, FrameAsk::Line {});
    assert_eq!(
        request(json!({"reason": "schedule"})).unwrap().ask,
        FrameAsk::Schedule {}
    );
    assert_eq!(
        request(json!({"reason": "focus", "look": "A notched wooden wheel."}))
            .unwrap()
            .ask,
        FrameAsk::Focus {
            look: "A notched wooden wheel.".to_owned()
        }
    );
    assert!(
        request(json!({"reason": "focus"})).is_err(),
        "a focus has a look"
    );
    assert!(request(json!({"reason": "schedule", "look": "x"})).is_err());
    assert!(request(json!({"reason": "whim"})).is_err());
}

#[test]
fn reason_and_moment_keys_are_their_serde_names() {
    for reason in [
        FrameReason::Line,
        FrameReason::Schedule,
        FrameReason::Inspection,
        FrameReason::Focus,
    ] {
        assert_eq!(json!(reason), json!(reason.key()));
    }
    for moment in [FrameMoment::Started, FrameMoment::Shown, FrameMoment::Ended] {
        assert_eq!(json!(moment), json!(moment.key()));
    }
    assert!(FrameReason::Focus.is_insert() && FrameReason::Inspection.is_insert());
    assert!(!FrameReason::Line.is_insert() && !FrameReason::Schedule.is_insert());
}

#[test]
fn the_schedule_s_slots_keep_their_cadence_and_a_late_tick_counts_every_slot_it_passed() {
    let began = Instant::now();
    let at = |seconds: u64| began + Duration::from_secs(seconds);
    let every = Duration::from_secs(30);
    assert_eq!(
        slots_due(began, every, 0, began),
        1,
        "slot 0 is due at once"
    );
    assert_eq!(slots_due(began, every, 1, at(29)), 0);
    assert_eq!(slots_due(began, every, 1, at(30)), 1);
    assert_eq!(
        slots_due(began, every, 2, at(95)),
        2,
        "slots 2 and 3 (60 s and 90 s) passed while the watch was busy"
    );
    assert_eq!(slots_due(began, every, 4, at(95)), 0);
    assert_eq!(
        slot_due(began, every, 4),
        at(120),
        "a slow frame never moves a slot"
    );
    assert_eq!(
        slots_due(began, Duration::ZERO, 0, at(3_600)),
        0,
        "every=0 makes none"
    );
    assert_eq!(
        slots_due(at(5), every, 0, began),
        1,
        "a clock before `began` is slot 0"
    );
}

#[test]
fn a_watch_line_is_a_speaker_and_a_line_or_a_focus_on_a_clue() {
    assert_eq!(
        WatchInput::parse("village_torchbearer | Look at this wheel."),
        WatchInput::Line {
            speaker: "village_torchbearer",
            line: "Look at this wheel."
        }
    );
    assert_eq!(
        WatchInput::parse("@focus A wooden wheel bears monkish signs.  "),
        WatchInput::Focus {
            item: None,
            text: "A wooden wheel bears monkish signs."
        }
    );
    // The id of the story's reference sheet of the thing comes before a bar.
    assert_eq!(
        WatchInput::parse("@focus brass_cipher_wheel | A brass wheel with a bright notch."),
        WatchInput::Focus {
            item: Some("brass_cipher_wheel"),
            text: "A brass wheel with a bright notch."
        }
    );
    for unreadable in [
        "@focus",
        "@focus   ",
        "no bar here",
        "@focusing on | it",
        "@focus  | words",
        "@focus wheel | ",
    ] {
        assert!(
            !matches!(WatchInput::parse(unreadable), WatchInput::Focus { .. }),
            "{unreadable}"
        );
    }
    assert_eq!(WatchInput::parse("@focus"), WatchInput::Unreadable);
    assert_eq!(WatchInput::parse("no bar here"), WatchInput::Unreadable);
}

#[test]
fn the_watch_reads_every_and_makes_frames_without_a_cap_unless_max_frames_is_given() {
    let watch = |text: &str| {
        let words: Vec<String> = text.split(' ').map(str::to_owned).collect();
        match parse(&words) {
            Ok(PersonaCommand::Watch(options)) => Ok(options),
            Ok(_) => panic!("{text} is a watch"),
            Err(error) => Err(error),
        }
    };
    let options = watch("watch frames=archive every=45").unwrap();
    assert_eq!(options.every, Some(Duration::from_secs(45)));
    assert_eq!(options.maximum_frames, u32::MAX);
    assert_eq!(options.parallel, MAXIMUM_PARALLEL_FRAMES);
    let options = watch("watch frames=archive max_frames=3 parallel=2").unwrap();
    assert_eq!(options.every, None, "the settings' every_seconds applies");
    assert_eq!(options.maximum_frames, 3);
    assert_eq!(options.parallel, 2);
    assert_eq!(
        watch("watch frames=archive every=0").unwrap().every,
        Some(Duration::ZERO)
    );
    for bad in [
        "watch frames=archive every=soon",
        "watch frames=archive every=3601",
        "watch every=45",
    ] {
        assert!(watch(bad).is_err(), "{bad} is refused");
    }
}

#[tokio::test]
async fn a_focus_is_an_insert_of_its_look_and_each_frame_moment_is_one_line() {
    use crate::dev_client::post_json;
    use crate::dev_http::DevReply;
    use crate::test_support::{STARTER_FILES, start};
    use crate::wire::EventsReply;
    let running = start(&STARTER_FILES).await;
    let port = running.port;
    let focus = json!({
        "picture": "A flooded scriptorium.",
        "lines": [],
        "speakers": speakers(),
        "ask": {"reason": "focus", "look": "A notched wooden wheel."}
    });
    let judged: FrameReply = post_json(port, PERSONA_FRAME, &focus).await.unwrap();
    let DevReply::Done { value } = judged else {
        panic!("a focus is judged without a model");
    };
    assert!(value.triggered && value.inspection);
    assert_eq!(value.shot, "insert");
    assert_eq!(value.next_picture, "A notched wooden wheel.");
    assert_eq!(value.inspecting, None);
    let mut empty = focus.clone();
    empty["ask"]["look"] = json!("  ");
    let refused: FrameReply = post_json(port, PERSONA_FRAME, &empty).await.unwrap();
    assert!(
        matches!(refused, DevReply::Rejected { .. }),
        "a focus has a look"
    );
    let timing = FrameTimingRequest {
        frame: "archive-2".to_owned(),
        moment: FrameMoment::Ended,
        reason: FrameReason::Focus,
        took_ms: 20_400,
    };
    let timed: FrameTimingReply = post_json(port, PERSONA_FRAME_TIMING, &timing)
        .await
        .unwrap();
    assert!(matches!(timed, DevReply::Done { .. }));
    let odd = FrameTimingRequest {
        frame: "../x y".to_owned(),
        ..timing
    };
    let refused: FrameTimingReply = post_json(port, PERSONA_FRAME_TIMING, &odd).await.unwrap();
    assert!(matches!(refused, DevReply::Rejected { .. }));
    let events = crate::client::get(port, "/events.json").await.unwrap();
    let lines: Vec<String> = serde_json::from_str::<EventsReply>(&events.body)
        .unwrap()
        .events
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("PERSONA "))
        .collect();
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("PERSONA frame_judged ")
                && line.contains(" shot=insert ")
                && line.contains(" reason=focus ")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|line| line
            .starts_with("PERSONA frame_timing moment=ended frame=archive-2 reason=focus at_ms=")
            && line.ends_with(" took_ms=20400")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "PERSONA refused verb=frame_timing error=bad_request"),
        "{lines:?}"
    );
}

#[test]
fn a_settings_file_written_before_the_schedule_has_the_built_in_every() {
    let mut shipped: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../assets/playground/persona/frames.json"
    ))
    .unwrap();
    shipped["show"]
        .as_object_mut()
        .unwrap()
        .remove("every_seconds");
    let read: FrameConfig = serde_json::from_value(shipped).unwrap();
    assert_eq!(read, FrameConfig::default());
}

#[tokio::test]
async fn the_watch_s_schedule_and_moments_become_the_toolbar_s_plate_timer_and_the_gap_is_logged() {
    use crate::dev_client::post_json;
    use crate::dev_http::DevReply;
    use crate::test_support::{STARTER_FILES, start};
    use crate::wire::EventsReply;
    let running = start(&STARTER_FILES).await;
    let port = running.port;
    let plate = || running.state.playground().snapshot().controls.plate;
    assert!(!plate().running, "no watch, no timer");
    let schedule = FrameScheduleRequest {
        running: true,
        every_ms: 30_000,
        due_in_ms: Some(30_000),
        missed: 0,
    };
    let told: FrameTimingReply = post_json(port, PERSONA_FRAME_SCHEDULE, &schedule)
        .await
        .unwrap();
    assert!(matches!(told, DevReply::Done { .. }));
    let shown = plate();
    assert!(shown.running && shown.every_ms == 30_000 && shown.due_at_ms.is_some());
    let moment = |frame: &str, moment: FrameMoment| FrameTimingRequest {
        frame: frame.to_owned(),
        moment,
        reason: FrameReason::Schedule,
        took_ms: 0,
    };
    for request in [
        moment("plates-1", FrameMoment::Started),
        moment("plates-1", FrameMoment::Shown),
        moment("plates-2", FrameMoment::Shown),
    ] {
        let timed: FrameTimingReply = post_json(port, PERSONA_FRAME_TIMING, &request)
            .await
            .unwrap();
        assert!(matches!(timed, DevReply::Done { .. }));
    }
    let shown = plate();
    assert!(!shown.making);
    assert_eq!(
        shown.last.map(|last| (last.frame, last.moment)),
        Some(("plates-2".to_owned(), FrameMoment::Shown))
    );
    let stop = FrameScheduleRequest {
        running: false,
        due_in_ms: None,
        missed: 1,
        ..schedule
    };
    let _: FrameTimingReply = post_json(port, PERSONA_FRAME_SCHEDULE, &stop)
        .await
        .unwrap();
    assert!(!plate().running && plate().missed == 1);
    let events = crate::client::get(port, "/events.json").await.unwrap();
    let lines: Vec<String> = serde_json::from_str::<EventsReply>(&events.body)
        .unwrap()
        .events
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("PERSONA frame_"))
        .collect();
    let has = |start: &str, part: &str| {
        lines
            .iter()
            .any(|line| line.starts_with(start) && line.contains(part))
    };
    assert!(
        has(
            "PERSONA frame_schedule running=true every_ms=30000 due_at_ms=",
            " missed=0"
        ),
        "{lines:?}"
    );
    assert!(
        has(
            "PERSONA frame_timing moment=started frame=plates-1 reason=schedule",
            " due_at_ms="
        ),
        "{lines:?}"
    );
    assert!(
        has(
            "PERSONA frame_timing moment=shown frame=plates-2 reason=schedule",
            " gap_ms="
        ),
        "{lines:?}"
    );
    assert!(
        has(
            "PERSONA frame_schedule running=false",
            " due_at_ms=- missed=1"
        ),
        "{lines:?}"
    );
}

// The screen keeps the host's own clock (`std::time::Instant`), so the slot is a short real one.
#[tokio::test]
async fn room_plates_go_up_one_slot_apart_oldest_first_and_a_long_queue_gives_way() {
    let every = Duration::from_millis(300);
    let screen = SharedScreen::default();
    screen.lock().unwrap().room_every = every;
    assert!(
        claim_room(&screen, 1, false).await,
        "the first plate goes up at once"
    );
    let started = Instant::now();
    let claim_at = async |number| {
        let claimed = claim_room(&screen, number, false).await;
        (claimed, started.elapsed())
    };
    // Plates 2 and 3 are ready together: each takes a slot, the older first.
    let ((two, two_at), (three, three_at)) = tokio::join!(claim_at(2), claim_at(3));
    assert!(two && three, "plates ready together are both shown");
    assert!(
        two_at >= every && two_at < three_at && three_at >= every * 2,
        "one slot apart, the older first: {two_at:?}, {three_at:?}"
    );
    // With two newer ones waiting, the oldest of three gives way.
    let ((four, _), (five, _), (six, _)) = tokio::join!(claim_at(4), claim_at(5), claim_at(6));
    assert!(!four, "the oldest of three waiting gives way");
    assert!(five && six);
    // Once a plate cued by a line is up, an older one would step back past the change.
    assert!(claim_room(&screen, 8, true).await);
    assert!(!claim_room(&screen, 7, false).await);
    // A plate whose frame is dropped while it waits holds back no newer one.
    let dropped = tokio::time::timeout(every / 3, claim_room(&screen, 9, false)).await;
    assert!(dropped.is_err(), "it was still waiting for its slot");
    let started = Instant::now();
    assert!(claim_room(&screen, 10, false).await);
    assert!(
        started.elapsed() < every * 3,
        "the next plate takes the next slot: {:?}",
        started.elapsed()
    );
}

#[test]
fn a_room_frame_is_told_how_each_thing_in_sight_looks() {
    let wheel = StoryItem {
        id: "brass_cipher_wheel".to_owned(),
        name: "Brass cipher wheel".to_owned(),
        place: "scriptorium".to_owned(),
        look: "A flat brass disc with engraved signs and one bright notch.".to_owned(),
        evidence: Some("cipher_wheel".to_owned()),
        picture: None,
    };
    let prompt = with_sheets("The flooded room.".to_owned(), &[wheel]);
    assert!(prompt.starts_with("The flooded room. "));
    assert!(prompt.ends_with(
        "Brass cipher wheel: A flat brass disc with engraved signs and one bright notch."
    ));
    assert_eq!(with_sheets("The room.".to_owned(), &[]), "The room.");
}

/// A reference sheet of the drowned archive's scriptorium, as `persona story plot` wrote it, with
/// the close-up its first insert kept.
fn scriptorium_sheet(id: &str, name: &str, look: &str) -> StoryItem {
    StoryItem {
        id: id.to_owned(),
        name: name.to_owned(),
        place: "scriptorium".to_owned(),
        look: look.to_owned(),
        evidence: None,
        picture: Some(format!("archive-scriptorium-{id}.png")),
    }
}

/// The Forge studio's limits, as its backend gives them.
fn forge_limits() -> BackendLimits {
    BackendLimits {
        maximum_text_characters: Some(FORGE_IMAGE_MAXIMUM_DESCRIPTION_CHARACTERS),
        maximum_negative_characters: Some(FORGE_IMAGE_MAXIMUM_NEGATIVE_CHARACTERS),
        maximum_pictures: MAXIMUM_PICTURES,
    }
}

/// The Codex image tool's limits, as its backend gives them.
fn codex_limits() -> BackendLimits {
    BackendLimits {
        maximum_pictures: CODEX_MAXIMUM_PICTURES,
        ..BackendLimits::REQUEST
    }
}

/// The five people of the drowned archive's scriptorium.
fn scriptorium_speakers() -> Vec<Speaker> {
    ["kael", "mira", "brann", "jory", "teren"]
        .map(|id| Speaker {
            id: id.to_owned(),
            display_name: id.to_owned(),
            job: "adventurer".to_owned(),
        })
        .to_vec()
}

#[test]
fn a_plate_attaches_people_first_then_the_things_close_ups_and_tells_the_rest_in_words() {
    let mut sheets = scriptorium_sheets();
    sheets[2].picture = None;
    let people: Vec<String> = (1..=5).map(|n| format!("ref-{n}.png")).collect();
    // Forge takes eight pictures: five people and both close-ups; the sheet without one is told.
    let fitted = fit_pictures(forge_limits().maximum_pictures, people.clone(), &sheets);
    assert_eq!(fitted.people, people);
    assert_eq!(
        fitted.things,
        [
            ThingReference {
                picture: "archive-scriptorium-ledger_leaves.png".to_owned(),
                shows: "Cut ledger leaves".to_owned(),
            },
            ThingReference {
                picture: "archive-scriptorium-brass_cipher_device.png".to_owned(),
                shows: "Old cipher wheel".to_owned(),
            },
        ]
    );
    assert_eq!(fitted.described, [sheets[2].clone()]);
    assert!(fitted.dropped.is_empty());
    let style = BriefStyle::default();
    let painting = Painting {
        prompt: "The scriptorium.",
        avoid: &style.avoid,
        sheets: &sheets,
    };
    let request = plate_request("archive-1", &painting, None, fitted);
    assert!(
        request
            .prompt
            .contains("Smeared catalogue: A small, water-swollen catalogue")
    );
    assert!(
        !request.prompt.contains("Old cipher wheel:"),
        "a close-up is not told"
    );
    // Codex takes five: the people fill them, so the close-ups are dropped and told in words.
    let fitted = fit_pictures(codex_limits().maximum_pictures, people.clone(), &sheets);
    assert_eq!(fitted.people, people);
    assert!(fitted.things.is_empty());
    assert_eq!(
        fitted.dropped,
        [
            "archive-scriptorium-ledger_leaves.png",
            "archive-scriptorium-brass_cipher_device.png"
        ]
    );
    assert_eq!(fitted.described.len(), 3);
    // An edit's base takes one, and people beyond the room are dropped too.
    let fitted = fit_pictures(codex_limits().maximum_pictures - 1, people, &[]);
    assert_eq!(fitted.people.len(), 4);
    assert_eq!(fitted.dropped, ["ref-5.png"]);
}

#[test]
fn the_judge_s_target_is_the_room_the_backend_leaves_with_a_margin_and_a_floor() {
    assert_eq!(picture_target(4000, 2000), 1800);
    assert_eq!(picture_target(4000, 3900), 400, "never under the floor");
}

/// The drowned archive's scriptorium (`development/market-threads/drowned-archive.beats`, live
/// run 2026-11-09): its three things' sheets.
fn scriptorium_sheets() -> Vec<StoryItem> {
    vec![
        scriptorium_sheet(
            "ledger_leaves",
            "Cut ledger leaves",
            concat!(
                "Several pale parchment leaves remain caught in the swollen ledger binding. Their ",
                "exposed edges are cleanly cut by a knife, with a faint, dull blue strip of ",
                "sealing wax caught in the fold. The parchment is wet-darkened, buckled and ",
                "matte, with softened, uneven edges; any ink on the leaves is faded and blurred ",
                "beyond ",
                "legibility."
            ),
        ),
        scriptorium_sheet(
            "brass_cipher_device",
            "Old cipher wheel",
            concat!(
                "A palm-sized brass wheel, round and slightly thick, with a shallow central hub ",
                "and a narrow rim. Its dull brown-gold surface is darkened in the recesses and ",
                "rubbed brighter along the edge. Three shallow settings are worn smooth in the ",
                "metal, and a narrow notch on the rim is polished brighter than the surrounding ",
                "brass. Fine scratches and small patches of tarnish mark its face."
            ),
        ),
        scriptorium_sheet(
            "catalogue",
            "Smeared catalogue",
            concat!(
                "A small, water-swollen catalogue lies among the fallen shelf and black ",
                "floodwater in the ruined scriptorium. Its warped leaves are dull, damp-edged and ",
                "ragged, ",
                "with a smear of dark ink across one margin. Beneath the smear, the shelf mark's ",
                "final stroke hooks left. Nearby entries show faded, blurred lines of ink that ",
                "cannot be read. The cover and page edges are stained and worn; no other marks ",
                "stand out."
            ),
        ),
    ]
}

/// The scriptorium's room plate on Forge: the judge's target leaves the whole text within what
/// the studio takes, with the house look's bans as its negative prompt.
#[test]
fn a_typical_room_plate_fits_what_the_forge_studio_takes() {
    let scene = concat!(
        "A flooded monastery scriptorium deep underground lit by one torch, knee-deep black ",
        "water between wooden writing desks, a fallen shelf of swollen ledgers in the aisle, an ",
        "open ledger of pages written in faded brown ink, with cut parchment edges, a brass ",
        "cipher wheel and a smeared catalogue leaf under a candle stub on a desk: at the left the ",
        "tall blond bearded man of about thirty-five with shoulder-length wavy hair, in a shaggy ",
        "grey-brown fur mantle over a worn wool tunic and mail, a sword at his hip, the ",
        "dark-haired woman of about forty with a long face and black hair under a dark green ",
        "wool hood and cloak, a leather belt and satchel and the broad red-bearded man of about ",
        "thirty with curly red hair, in a heavy brown wool travelling cloak over a leather ",
        "jerkin wade in; in the middle the thin village youth of about sixteen with short ",
        "tousled light-brown hair and a narrow freckled face, in a patched brown wool tunic with ",
        "a rope belt holds up the torch; at the right the young woman of about twenty with a ",
        "single long brown braid and freckles, in a scuffed brown leather jerkin over a cream ",
        "linen shirt leans over the open ledger."
    );
    let sheets = scriptorium_sheets();
    let limits = forge_limits();
    let options =
        match parse(&["watch", "frames=archive-scriptorium", "edit=false"].map(str::to_owned)) {
            Ok(PersonaCommand::Watch(options)) => options,
            other => panic!("a watch: {other:?}"),
        };
    let watch = FrameWatch::new(options, &scriptorium_speakers(), "archive-scriptorium.png");
    let fixed = watch
        .fixed_characters(&limits, &sheets)
        .expect("a valid plate");
    let target = picture_target(FORGE_IMAGE_MAXIMUM_DESCRIPTION_CHARACTERS, fixed);
    // A picture of the scene at its target: the scene, made as long as the target.
    let at_target: String = scene.chars().cycle().take(target).collect();
    let style = BriefStyle::default();
    let wide = "a wide shot of the whole room, to show the crowd and where everyone is";
    let prompt = painted_prompt(&style, &at_target, wide);
    let painting = Painting {
        prompt: &prompt,
        avoid: &style.avoid,
        sheets: &sheets,
    };
    let people: Vec<String> = scriptorium_speakers()
        .iter()
        .map(|speaker| format!("ref-{}.png", speaker.id))
        .collect();
    let fitted = fit_pictures(limits.maximum_pictures, people, &sheets);
    assert_eq!(fitted.things.len(), 3, "every close-up is attached");
    let request = plate_request("archive-scriptorium-1", &painting, None, fitted)
        .checked()
        .expect("a valid plate request");
    let sent_length = request.sent_text(&limits).encode_utf16().count();
    assert!(
        sent_length <= FORGE_IMAGE_MAXIMUM_DESCRIPTION_CHARACTERS,
        "the text sent is {sent_length} characters"
    );
    let negative = request.negative_text(&limits).expect("the bans");
    assert!(negative.contains("readable text") && !request.sent_text(&limits).contains("HDR"));
    // On Codex the same plate keeps the bans in its text.
    assert!(
        request
            .sent_text(&codex_limits())
            .contains("Never show: painting")
    );
}

#[tokio::test]
async fn a_picture_over_its_target_is_asked_again_with_why_and_never_cut() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use df_ai::text::TextOutcome;
    use df_ai::text_fixture::ScriptedBackend;

    use crate::dev_client::post_json;
    use crate::dev_http::DevReply;
    use crate::test_support::{STARTER_FILES, start};
    use crate::text::{BackendChoice, TextRuntime};

    let running = start(&STARTER_FILES).await;
    let answer = |picture: &str| {
        json!({
            "changed_percent": 40, "changes": [], "next_picture": picture, "shot": "wide",
            "inspected_item": null, "new_look": false
        })
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let script = ScriptedBackend::new(
        [
            answer(&"x".repeat(120)),
            answer("The woman in grey lifts the lamp."),
        ]
        .into_iter()
        .map(TextOutcome::Answered),
        Arc::clone(&calls),
    )
    .recording(Arc::clone(&asked));
    let recorder = Arc::clone(&running.state);
    let text = TextRuntime::new(
        BackendChoice::Scripted(Arc::new(script), None),
        Arc::new(move |line| recorder.record_line(line)),
    );
    let app = super::frame::router_with(Arc::clone(&running.state), Arc::new(text));
    let listener = crate::server::bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        // The server runs until the test's runtime stops; its end is the test's end.
        let _ = axum::serve(listener, app).await;
    });
    let request = FrameRequest {
        picture: "A flooded scriptorium.".to_owned(),
        lines: vec!["Lena: Look at this.".to_owned()],
        speakers: speakers(),
        gate_percent: None,
        model: None,
        last_shot: None,
        peak_percent: 0,
        last_inspected: None,
        ask: FrameAsk::Line {},
        picture_target_characters: Some(100),
    };
    let judged: FrameReply = post_json(port, PERSONA_FRAME, &request).await.unwrap();
    let DevReply::Done { value } = judged else {
        panic!("judged: {judged:?}");
    };
    assert_eq!(value.next_picture, "The woman in grey lifts the lamp.");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let asked = asked.lock().unwrap();
    assert_eq!(asked[0].facts["next_picture_target"], 100);
    let why = asked[1].facts["previous_refusal"]
        .as_str()
        .unwrap_or_default();
    assert!(
        why.contains("120 characters, over its target of 100"),
        "{why}"
    );
    let retried = running
        .state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .any(|line| {
            line.text
                == "PERSONA frame_retried reason=picture_over_target characters=120 target=100"
        });
    assert!(retried);
}

#[test]
fn room_frames_start_ahead_of_their_slots_to_cover_how_long_one_takes_to_make() {
    let every = Duration::from_secs(30);
    let slow = Some(Duration::from_secs(145));
    // Making one spans five slots: seven are kept in the making.
    assert!(needs_lead(6, slow, every, Duration::from_secs(15)));
    assert!(!needs_lead(7, slow, every, Duration::from_secs(15)));
    assert!(
        !needs_lead(2, slow, every, Duration::from_secs(10)),
        "half a slot between starts"
    );
    assert!(
        !needs_lead(0, None, every, Duration::from_secs(60)),
        "none before one is made"
    );
    assert!(
        !needs_lead(0, slow, Duration::ZERO, Duration::from_secs(60)),
        "no schedule"
    );
}

#[tokio::test]
async fn a_room_plate_is_drawn_after_the_reference_sheets_the_library_holds() {
    use crate::test_support::{STARTER_FILES, start};
    let running = start(&STARTER_FILES).await;
    running.write_asset("media/ref-warden.png", "a sheet");
    running.write_asset("media/ref-ferryman.png", "a sheet");
    let character = |name: &str| CharacterRequest {
        name: name.to_owned(),
        display_name: None,
        description: None,
        portrait: None,
    };
    let cast = [
        character("ferryman"),
        character("rival"),
        character("warden"),
    ];
    assert_eq!(
        reference_sheets(running.port, &cast)
            .await
            .expect("the library answers"),
        ["ref-ferryman.png", "ref-warden.png"],
        "in the cast's order, without the one who has no sheet"
    );
}
