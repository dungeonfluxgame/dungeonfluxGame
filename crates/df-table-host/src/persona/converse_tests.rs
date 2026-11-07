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

#[test]
fn the_character_a_line_names_answers_by_id_display_name_or_given_name() {
    let table = table();
    for (line, answers) in [
        ("Which road, Aldric?", "innkeeper"),
        ("Ask the innkeeper.", "elf"),
        ("What does the wizard say?", "wizard"),
        ("Hand it to the braided woman, would you?", "braided_woman"),
    ] {
        let history = [turn("elf", line)];
        let by_elf = table.next_speaker(&history).id.clone();
        // The speaker is never the one who named themself; the named one answers.
        if answers == "elf" {
            assert_ne!(by_elf, "elf", "{line}");
        } else {
            assert_eq!(by_elf, answers, "{line}");
        }
    }
}

#[test]
fn the_first_name_in_a_line_is_the_one_who_answers() {
    let table = table();
    let history = [turn("elf", "Wizard, ask Aldric about the road.")];
    assert_eq!(table.next_speaker(&history).id, "wizard");
}

#[test]
fn a_name_must_be_a_whole_word() {
    let table = table();
    let history = [turn("elf", "The wizardry of it is beyond me.")];
    // No one is named, so the quietest speaker takes the turn.
    assert_ne!(table.next_speaker(&history).id, "elf");
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
