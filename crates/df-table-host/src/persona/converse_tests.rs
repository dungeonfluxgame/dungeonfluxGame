use std::time::Duration;

use super::cli::{PersonaCommand, parse};
use super::converse::{
    ConverseError, ConverseRequest, DEFAULT_TURNS, MAXIMUM_TURNS, Speaker, Table, Turn, pause_after,
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

/// The speaker `table` picks after `history`.
fn next_after(table: &Table, history: &[Turn]) -> String {
    table.next_speaker(history).id.clone()
}

#[test]
fn the_character_a_line_names_answers_by_id_display_name_or_given_name() {
    let table = table();
    // Seats: elf, innkeeper, braided_woman, wizard. With nobody named the leftmost speaker who is
    // not the last one answers, so each case names someone else to show the name did the choosing.
    for (by, line, answers) in [
        ("elf", "What does the wizard say?", "wizard"),
        (
            "elf",
            "Hand it to the braided woman, would you?",
            "braided_woman",
        ),
        ("wizard", "Ask the innkeeper.", "innkeeper"),
        ("wizard", "Which road, Aldric?", "innkeeper"),
        ("braided_woman", "Wizard, a tale!", "wizard"),
    ] {
        assert_eq!(
            next_after(&table, &[turn(by, line)]),
            answers,
            "{by}: {line}"
        );
    }
}

#[test]
fn with_nobody_named_the_leftmost_of_the_quietest_answers() {
    let table = table();
    assert_eq!(
        next_after(&table, &[turn("elf", "A quiet evening.")]),
        "innkeeper"
    );
    assert_eq!(
        next_after(&table, &[turn("innkeeper", "A quiet evening.")]),
        "elf"
    );
    // Everyone has spoken but the wizard, who has been silent longest of all.
    let history = [
        turn("elf", "One."),
        turn("innkeeper", "Two."),
        turn("braided_woman", "Three."),
    ];
    assert_eq!(next_after(&table, &history), "wizard");
}

#[test]
fn the_first_name_in_a_line_is_the_one_who_answers() {
    let table = table();
    let history = [turn("elf", "Wizard, ask Aldric about the road.")];
    assert_eq!(next_after(&table, &history), "wizard");
    let history = [turn("elf", "Aldric, ask the wizard about the road.")];
    assert_eq!(next_after(&table, &history), "innkeeper");
}

#[test]
fn a_name_must_be_a_whole_word() {
    let table = table();
    // "wizardry" is not "wizard": nobody is named, so the leftmost quiet speaker answers.
    let history = [turn("elf", "The wizardry of it is beyond me.")];
    assert_eq!(next_after(&table, &history), "innkeeper");
    let history = [turn("elf", "My aldrich ring is lost.")];
    assert_eq!(next_after(&table, &history), "innkeeper");
}

#[test]
fn a_line_that_names_only_the_speaker_names_nobody_else() {
    let table = table();
    let history = [turn("wizard", "I, the wizard, am tired.")];
    assert_eq!(next_after(&table, &history), "elf");
}

#[test]
fn ids_with_hyphens_names_with_apostrophes_and_titles_match_as_words() {
    let table = Table::new(vec![
        speaker("elf", "The Elf"),
        speaker("braided-woman", "Mother O'Neill"),
        speaker("tom", "Old Tom"),
        speaker("wizard", "The Wizard"),
    ])
    .unwrap();
    // The id's words, with the hyphen read as a space.
    assert_eq!(
        next_after(&table, &[turn("wizard", "Braided woman, your thread?")]),
        "braided-woman"
    );
    // The display name split the way a line is split: "O'Neill" is "o" and "neill".
    assert_eq!(
        next_after(&table, &[turn("wizard", "Mother O'Neill, your thread?")]),
        "braided-woman"
    );
    // "old" is a title, not Tom's given name: the old road names nobody, so the leftmost answers.
    assert_eq!(
        next_after(&table, &[turn("wizard", "Take the old road.")]),
        "elf"
    );
    assert_eq!(
        next_after(&table, &[turn("wizard", "Tom, take the road.")]),
        "tom"
    );
}

#[test]
fn the_longer_name_wins_where_two_characters_share_a_given_name() {
    let table = Table::new(vec![
        speaker("elf", "The Elf"),
        speaker("keeper", "Aldric Venn"),
        speaker("smith", "Aldric Stone"),
    ])
    .unwrap();
    assert_eq!(
        next_after(&table, &[turn("elf", "Aldric Stone, a word?")]),
        "smith"
    );
    assert_eq!(
        next_after(&table, &[turn("elf", "Aldric Venn, a word?")]),
        "keeper"
    );
    // The bare given name is shared: the seat furthest left answers.
    assert_eq!(
        next_after(&table, &[turn("elf", "Aldric, a word?")]),
        "keeper"
    );
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
    assert_eq!(next_after(&table, &history), "keeper");
}

#[test]
fn someone_who_spoke_in_the_last_three_turns_lets_the_quietest_answer() {
    let table = table();
    let history = [
        turn("wizard", "A tale for the road."),
        turn("innkeeper", "Coin first, friend."),
        turn("braided_woman", "Mend your sleeve, wizard."),
    ];
    // The braided woman named the wizard, who spoke two turns ago: the elf, silent so far, answers.
    assert_eq!(table.next_speaker(&history).id, "elf");
}

#[test]
fn the_next_speaker_is_never_the_last_one() {
    let table = table();
    let history = [turn("innkeeper", "Which road, friend?")];
    assert_ne!(table.next_speaker(&history).id, "innkeeper");
}

#[test]
fn a_situation_tells_the_setting_the_topic_the_latest_lines_and_the_task() {
    let table = table();
    let speaker = table.opener(Some("wizard")).unwrap();
    let opening = table.situation(speaker, Some("the guild road"), &[]);
    assert!(opening.contains("Aldric Venn"));
    assert!(opening.contains("You are The Wizard"));
    assert!(opening.contains("the guild road"));
    assert!(opening.contains("You speak first"));
    let history: Vec<Turn> = (0..9)
        .map(|index| turn("elf", &format!("line number {index}")))
        .collect();
    let later = table.situation(speaker, None, &history);
    assert!(later.contains("The Elf: line number 8"));
    assert!(later.contains("line number 3"));
    assert!(
        !later.contains("line number 2"),
        "only the latest six lines"
    );
    assert!(later.contains("React to the last thing said"));
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
