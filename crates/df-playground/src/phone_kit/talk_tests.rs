//! The Talk panel's decisions: whom the hero addresses and the thread as turns.

use crate::lobby::model::BubbleTone;
use crate::phone_kit::talk::{TalkPanel, ThreadLine};

fn line(speaker: &str, voice: BubbleTone, text: &str) -> ThreadLine {
    ThreadLine {
        speaker: speaker.to_owned(),
        voice,
        text: text.to_owned(),
    }
}

#[test]
fn the_address_line_names_the_person_or_the_room() {
    let mut panel = TalkPanel::default();
    assert_eq!(panel.addressing_text(), "Speaking to the room");
    panel.addressing = Some("Mara Voss".to_owned());
    assert_eq!(panel.addressing_text(), "Speaking to Mara Voss");
    assert!(panel.is_addressed("Mara Voss"));
    assert!(!panel.is_addressed("Dain Rusk"));
}

#[test]
fn neighbouring_lines_of_one_speaker_are_one_turn_with_the_name_once() {
    let panel = TalkPanel {
        thread: vec![
            line("Rylan", BubbleTone::Hero, "A room, please."),
            line("Mara Voss", BubbleTone::Npc, "Rooms I have."),
            line("Mara Voss", BubbleTone::Npc, "Quiet words cost extra."),
            line("Rylan", BubbleTone::Hero, "How much?"),
        ],
        ..TalkPanel::default()
    };
    let turns = panel.turns();
    assert_eq!(turns.len(), 3);
    assert_eq!(turns[1].speaker, "Mara Voss");
    assert_eq!(
        turns[1].lines,
        vec!["Rooms I have.", "Quiet words cost extra."]
    );
    assert_eq!(turns[2].voice, BubbleTone::Hero);
}
