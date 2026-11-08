use std::time::Duration;

use serde_json::json;

use super::address::{AddressError, address_schema, parse_address};
use super::cli::{PersonaCommand, parse};
use super::converse::{
    ConverseError, ConverseRequest, DEFAULT_TURNS, Happening, MAXIMUM_TURNS, SceneEvent, Speaker,
    Table, Turn, parse_events, pause_after,
};
use crate::speech::{SpeechError, Voiced};

fn speaker(id: &str, display_name: &str) -> Speaker {
    Speaker {
        id: id.to_owned(),
        display_name: display_name.to_owned(),
        job: "a traveller".to_owned(),
    }
}

fn table() -> Table {
    Table::new(vec![
        speaker("elf", "The Elf"),
        speaker("innkeeper", "Aldric Venn"),
        speaker("braided_woman", "The Braided Woman"),
        speaker("wizard", "The Wizard"),
    ])
    .expect("a table of four")
}

fn turn(speaker: &str, line: &str) -> Turn {
    Turn {
        speaker: speaker.to_owned(),
        line: line.to_owned(),
    }
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

fn request_of(text: &str) -> ConverseRequest {
    match parse(&words(text)).expect("parses") {
        PersonaCommand::Converse(request) => request,
        other => panic!("not a converse command: {other:?}"),
    }
}

#[test]
fn a_table_needs_two_characters_and_an_opener_must_sit_at_it() {
    assert_eq!(
        Table::new(vec![speaker("elf", "The Elf")]),
        Err(ConverseError::TooFewSpeakers { found: 1 })
    );
    let table = table();
    assert_eq!(table.opener(None).unwrap().id, "elf");
    assert_eq!(table.opener(Some("wizard")).unwrap().id, "wizard");
    assert_eq!(
        table.opener(Some("dragon")),
        Err(ConverseError::UnknownOpener {
            opener: "dragon".to_owned()
        })
    );
}

/// The speaker `table` picks after `history` when the last line was directed at `addressed`.
fn next_after(table: &Table, history: &[Turn], addressed: Option<&str>) -> String {
    table.next_speaker(history, addressed).id.clone()
}

#[test]
fn the_character_the_last_line_was_directed_at_answers() {
    let table = table();
    // Seats: elf, innkeeper, braided_woman, wizard. With nobody addressed the leftmost speaker who
    // is not the last one answers, so each case addresses someone else to show it did the choosing.
    for (by, addressed) in [
        ("elf", "wizard"),
        ("elf", "braided_woman"),
        ("wizard", "innkeeper"),
        ("braided_woman", "wizard"),
    ] {
        assert_eq!(
            next_after(&table, &[turn(by, "A line.")], Some(addressed)),
            addressed,
            "{by} addressed {addressed}"
        );
    }
}

#[test]
fn with_nobody_addressed_the_leftmost_of_the_quietest_answers() {
    let table = table();
    assert_eq!(
        next_after(&table, &[turn("elf", "A quiet evening.")], None),
        "innkeeper"
    );
    assert_eq!(
        next_after(&table, &[turn("innkeeper", "A quiet evening.")], None),
        "elf"
    );
    // Everyone has spoken but the wizard, who has been silent longest of all.
    let history = [
        turn("elf", "One."),
        turn("innkeeper", "Two."),
        turn("braided_woman", "Three."),
    ];
    assert_eq!(next_after(&table, &history, None), "wizard");
}

#[test]
fn the_speaker_and_an_id_not_at_the_table_are_never_the_one_who_answers() {
    let table = table();
    let history = [turn("wizard", "I am tired.")];
    assert_eq!(next_after(&table, &history, Some("wizard")), "elf");
    assert_eq!(next_after(&table, &history, Some("dragon")), "elf");
}

#[test]
fn at_a_table_of_two_the_other_always_answers_even_inside_the_cooldown() {
    let table = Table::new(vec![
        speaker("elf", "The Elf"),
        speaker("keeper", "Aldric Venn"),
    ])
    .unwrap();
    let history = [
        turn("elf", "Aldric?"),
        turn("keeper", "Yes, elf?"),
        turn("elf", "Aldric, the road?"),
    ];
    assert_eq!(next_after(&table, &history, Some("keeper")), "keeper");
}

#[test]
fn someone_who_spoke_in_the_last_three_turns_lets_the_quietest_answer() {
    let table = table();
    let history = [
        turn("wizard", "A tale for the road."),
        turn("innkeeper", "Coin first, friend."),
        turn("braided_woman", "Mend your sleeve, wizard."),
    ];
    // The braided woman addressed the wizard, who spoke two turns ago: the elf, silent so far,
    // answers.
    assert_eq!(table.next_speaker(&history, Some("wizard")).id, "elf");
}

#[test]
fn the_next_speaker_is_never_the_last_one() {
    let table = table();
    let history = [turn("innkeeper", "Which road, friend?")];
    assert_ne!(table.next_speaker(&history, None).id, "innkeeper");
    assert_ne!(
        table.next_speaker(&history, Some("innkeeper")).id,
        "innkeeper"
    );
}

#[test]
fn a_situation_tells_the_setting_the_topic_the_latest_lines_and_the_task() {
    let table = table();
    let speaker = table.opener(Some("wizard")).unwrap();
    let opening = table.situation(speaker, Some("the guild road"), None, &[]);
    // The others are there by who they are in the scene, never by a name nobody gave.
    assert!(opening.contains("innkeeper (a traveller)"));
    assert!(!opening.contains("Aldric Venn"));
    assert!(opening.contains("You are The Wizard"));
    assert!(opening.contains("the guild road"));
    assert!(opening.contains("You speak first"));
    let history: Vec<Turn> = (0..9)
        .map(|index| turn("elf", &format!("line number {index}")))
        .collect();
    let later = table.situation(speaker, None, Some("A tankard falls."), &history);
    assert!(later.contains("Happening right now, as you speak: A tankard falls."));
    assert!(later.contains("elf: line number 8"));
    assert!(!later.contains("The Elf"));
    assert!(later.contains("line number 3"));
    assert!(
        !later.contains("line number 2"),
        "only the latest six lines"
    );
    assert!(later.contains("React first to what is happening right now (A tankard falls.)"));
    let quiet = table.situation(speaker, None, None, &history);
    assert!(quiet.contains("React to the last thing said"));
}

#[test]
fn a_line_waits_for_its_reading_time_or_its_recording_whichever_is_longer() {
    let short = pause_after("Hello.", None);
    assert!(short >= Duration::from_millis(1_800), "{short:?}");
    let long_text = "word ".repeat(60);
    let reading = pause_after(&long_text, None);
    assert!(reading > short);
    let played = Voiced::Played {
        clip: "voice-1.mp3".to_owned(),
        bytes: 40_000,
        voice_profile: "model/character".to_owned(),
        synthesis_ms: 400,
    };
    // 40 000 bytes of the 32 kbps recording are ten seconds of speech.
    assert_eq!(pause_after("Hi.", Some(&played)), Duration::from_secs(10));
    let failed = Voiced::Failed {
        error: SpeechError::Stopped,
    };
    assert_eq!(pause_after("Hi.", Some(&failed)), pause_after("Hi.", None));
}

#[test]
fn the_command_reads_its_options_and_an_optional_topic() {
    let plain = request_of("converse");
    assert_eq!(plain.turns, DEFAULT_TURNS);
    assert_eq!(plain.opener, None);
    assert_eq!(plain.topic, None);
    assert!(plain.pace && !plain.speak);
    let full = request_of("converse turns=12 opener=wizard speak=true pace=false the guild road");
    assert_eq!(full.turns, 12);
    assert_eq!(full.opener.as_deref(), Some("wizard"));
    assert!(full.speak && !full.pace);
    assert_eq!(full.topic.as_deref(), Some("the guild road"));
}

#[test]
fn an_events_file_reads_turns_and_happenings_in_order_and_names_a_bad_line() {
    let events = parse_events(
        "# a scene
3 | Kael stands.

0 | A drunk stumbles in.
2 | ! persona state elf reset
",
    )
    .unwrap();
    assert_eq!(
        events,
        [
            SceneEvent {
                turn: 0,
                happens: Happening::Seen("A drunk stumbles in.".to_owned())
            },
            SceneEvent {
                turn: 2,
                happens: Happening::Command(
                    ["persona", "state", "elf", "reset"]
                        .map(str::to_owned)
                        .to_vec()
                )
            },
            SceneEvent {
                turn: 3,
                happens: Happening::Seen("Kael stands.".to_owned())
            },
        ]
    );
    assert_eq!(events[1].seen(), None);
    assert_eq!(
        parse_events(
            "1 | fine
not an event
"
        ),
        Err(ConverseError::InvalidEvent { line: 2 })
    );
    assert_eq!(
        parse_events(
            "1 | !
"
        ),
        Err(ConverseError::InvalidEvent { line: 1 })
    );
}

#[test]
fn frame_options_are_read_with_frames_and_refused_without_it() {
    assert_eq!(request_of("converse").frames, None);
    let watched = request_of(
        "converse frames=bar-talk gate=40 edit_from=last hold=10 max_frames=3 judge_model=luna",
    )
    .frames
    .expect("frames");
    assert_eq!(watched.name, "bar-talk");
    assert_eq!(watched.gate_percent, Some(40));
    assert!(watched.edit && watched.from_last);
    assert_eq!(watched.hold, Duration::from_secs(10));
    assert_eq!(watched.maximum_frames, 3);
    assert_eq!(watched.judge_model.as_deref(), Some("luna"));
    assert!(
        !request_of("converse frames=x edit=false")
            .frames
            .expect("frames")
            .edit
    );
    for bad in [
        "gate=30",
        "frames=x gate=101",
        "frames=x edit_from=middle",
        "frames=bad.name",
        "frames=x parallel=0",
        "frames=x parallel=5",
    ] {
        assert!(
            parse(&words(&format!("converse {bad}"))).is_err(),
            "{bad} is refused"
        );
    }
}

#[test]
fn the_command_refuses_a_turn_count_that_is_not_a_whole_number_in_range() {
    for bad in ["turns=0", "turns=x", "turns=-3", "turns=10001"] {
        assert!(
            parse(&words(&format!("converse {bad}"))).is_err(),
            "{bad} is refused"
        );
    }
    assert_eq!(
        request_of(&format!("converse turns={MAXIMUM_TURNS}")).turns,
        MAXIMUM_TURNS
    );
}

fn listed() -> Vec<Speaker> {
    table().speakers().to_vec()
}

#[test]
fn an_answer_naming_a_listed_other_speaker_is_read() {
    assert_eq!(
        parse_address(&json!({"addressed": "wizard"}), "elf", &listed()),
        Ok(Some("wizard".to_owned()))
    );
}

#[test]
fn a_null_answer_is_a_line_to_the_room() {
    assert_eq!(
        parse_address(&json!({"addressed": null}), "elf", &listed()),
        Ok(None)
    );
}

#[test]
fn an_answer_naming_the_speaker_is_refused() {
    assert_eq!(
        parse_address(&json!({"addressed": "elf"}), "elf", &listed()),
        Err(AddressError::AddressedSelf {
            id: "elf".to_owned()
        })
    );
}

#[test]
fn an_answer_naming_nobody_listed_is_refused() {
    assert_eq!(
        parse_address(&json!({"addressed": "dragon"}), "elf", &listed()),
        Err(AddressError::UnknownAddressed {
            id: "dragon".to_owned()
        })
    );
}

#[test]
fn a_malformed_answer_is_refused() {
    for answer in [
        json!("wizard"),
        json!({}),
        json!({"addressed": 3}),
        json!({"addressed": ["wizard"]}),
    ] {
        assert!(
            matches!(
                parse_address(&answer, "elf", &listed()),
                Err(AddressError::MalformedAnswer { .. })
            ),
            "{answer}"
        );
    }
}

#[test]
fn the_schema_allows_the_listed_ids_and_null() {
    let schema = address_schema(&listed());
    let allowed = &schema["properties"]["addressed"]["enum"];
    assert_eq!(
        allowed,
        &json!(["elf", "innkeeper", "braided_woman", "wizard", null])
    );
}

// TODO(DEV-12): `converse` stops with the lines so far when the address route fails, and the pause
// and the route call run together (a faked host with a slow route).
