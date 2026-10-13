//! Which text of a display view is spoken, and the identity of one spoken line.
//!
//! The server registers these lines when it sends a display view and the browser asks for
//! them by key, so both sides use this one definition and cannot drift apart.
use df_protocol::common as rpc;
use sha2::{Digest, Sha256};

/// Role of the scene narration, and of a dialogue line that has no named speaker.
pub(crate) const NARRATOR_ROLE: &str = "narrator";
const NPC_ROLE_PREFIX: &str = "npc:";
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const MAXIMUM_VOICE_CLIP_CHARACTERS: usize = 600;

/// One piece of server-authored text a display may have spoken aloud.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VoiceLine {
    pub(crate) role: String,
    pub(crate) text: String,
}

impl VoiceLine {
    /// Lowercase hexadecimal SHA-256 of `role`, a newline and `text`.
    pub(crate) fn key(&self) -> String {
        self.digest()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub(crate) fn digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(self.role.as_bytes());
        hasher.update(b"\n");
        hasher.update(self.text.as_bytes());
        hasher.finalize().into()
    }

    /// Whether the line belongs to a named speaker rather than the narrator (the server's
    /// voice cast reads this; the browser asks for `speaker` directly).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn is_npc(&self) -> bool {
        self.speaker().is_some()
    }

    /// The speaker name of a named-speaker line; `None` for the narrator.
    pub(crate) fn speaker(&self) -> Option<&str> {
        self.role.strip_prefix(NPC_ROLE_PREFIX)
    }
}

/// Splits a line into provider-sized clips, preferring sentence boundaries and then words.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn voice_segments(text: &str) -> Vec<String> {
    if text.chars().count() <= MAXIMUM_VOICE_CLIP_CHARACTERS {
        return vec![text.to_owned()];
    }
    let mut sentences = Vec::new();
    let mut start = 0;
    for (index, character) in text.char_indices() {
        let end = index + character.len_utf8();
        if matches!(character, '.' | '!' | '?')
            && text[end..].chars().next().is_some_and(char::is_whitespace)
        {
            let sentence = text[start..end].trim();
            if !sentence.is_empty() {
                sentences.push(sentence.to_owned());
            }
            start = end;
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        sentences.push(tail.to_owned());
    }

    let mut segments = Vec::new();
    let mut current = String::new();
    for sentence in sentences {
        let words = sentence.split_whitespace();
        for word in words {
            let mut characters = word.chars();
            while characters.clone().count() > MAXIMUM_VOICE_CLIP_CHARACTERS {
                let chunk: String = characters
                    .by_ref()
                    .take(MAXIMUM_VOICE_CLIP_CHARACTERS)
                    .collect();
                push_voice_segment(&mut segments, &mut current, &chunk);
            }
            let word: String = characters.collect();
            if !word.is_empty() {
                push_voice_segment(&mut segments, &mut current, &word);
            }
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

#[cfg(not(target_arch = "wasm32"))]
fn push_voice_segment(segments: &mut Vec<String>, current: &mut String, word: &str) {
    let separator = usize::from(!current.is_empty());
    if current.chars().count() + separator + word.chars().count() > MAXIMUM_VOICE_CLIP_CHARACTERS {
        segments.push(std::mem::take(current));
    }
    if !current.is_empty() {
        current.push(' ');
    }
    current.push_str(word);
}

/// The scene narration first, then the current speaker line; empty text is not spoken. The
/// scene's rules note is a separate field and is never read here: it is shown, not voiced.
pub(crate) fn display_lines(display: &rpc::DisplayGameplayView) -> Vec<VoiceLine> {
    let mut lines = Vec::with_capacity(2);
    if !display.narration.trim().is_empty() {
        lines.push(VoiceLine {
            role: NARRATOR_ROLE.to_owned(),
            text: display.narration.clone(),
        });
    }
    if let Some(journey) = display.journey.as_ref()
        && !journey.dialogue.trim().is_empty()
    {
        let role = if journey.speaker.trim().is_empty() {
            NARRATOR_ROLE.to_owned()
        } else {
            format!("{NPC_ROLE_PREFIX}{}", journey.speaker)
        };
        lines.push(VoiceLine {
            role,
            text: journey.dialogue.clone(),
        });
    }
    lines.extend(conversation_reply(display));
    lines.extend(fight_lines(display));
    lines
}

/// A battle cry or Dungeon Master fight line as a speakable line, in outcome order with
/// the finished fight's closing line last. Hero cries are voiced as their named speaker
/// (the cast falls back to the neutral voice for speakers with no entry); empty text is
/// never spoken. Both sides share this definition with the stage script, so the browser
/// can only voice a line the server registered.
pub(crate) fn fight_voice_line(line: &rpc::FightLine) -> Option<VoiceLine> {
    if line.text.trim().is_empty() {
        return None;
    }
    let role = match rpc::TableRole::try_from(line.role) {
        Ok(rpc::TableRole::Narrator) => NARRATOR_ROLE.to_owned(),
        Ok(rpc::TableRole::Hero) | Ok(rpc::TableRole::Npc) if !line.speaker.trim().is_empty() => {
            format!("{NPC_ROLE_PREFIX}{}", line.speaker.trim())
        }
        _ => return None,
    };
    Some(VoiceLine {
        role,
        text: line.text.clone(),
    })
}

fn fight_lines(display: &rpc::DisplayGameplayView) -> Vec<VoiceLine> {
    let Some(journey) = display.journey.as_ref() else {
        return Vec::new();
    };
    let Some(combat) = journey.combat.as_ref() else {
        return Vec::new();
    };
    combat
        .outcomes
        .iter()
        .flat_map(|outcome| outcome.lines.iter())
        .chain(combat.closing_lines.iter())
        .filter_map(fight_voice_line)
        .collect()
}

/// The speaker's reply to a hero's typed words, while it is still the last thing said at the
/// table. Any accepted action (and so any change of beat) logs a later line, which ends it, so
/// an old reply is never spoken again in a later beat. It is voiced as that speaker, the same
/// cast voice as the speaker's scripted line.
fn conversation_reply(display: &rpc::DisplayGameplayView) -> Option<VoiceLine> {
    let last = display.journey.as_ref()?.table_log.last()?;
    let spoken_by_npc = rpc::TableRole::try_from(last.role) == Ok(rpc::TableRole::Npc)
        && rpc::TableKind::try_from(last.kind) == Ok(rpc::TableKind::Speech);
    let (speaker, text) = (last.speaker.trim(), last.text.trim());
    (spoken_by_npc && !speaker.is_empty() && !text.is_empty()).then(|| VoiceLine {
        role: format!("{NPC_ROLE_PREFIX}{speaker}"),
        text: text.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(narration: &str, speaker: &str, dialogue: &str) -> rpc::DisplayGameplayView {
        rpc::DisplayGameplayView {
            narration: narration.to_owned(),
            journey: Some(rpc::JourneyView {
                speaker: speaker.to_owned(),
                dialogue: dialogue.to_owned(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn key_is_the_sha256_of_role_newline_text() {
        let line = VoiceLine {
            role: "narrator".to_owned(),
            text: "hello".to_owned(),
        };

        assert_eq!(
            line.key(),
            "c803b9299d97b0114e79ffaaf7ce73254cad145d8e6d7b917aea1e23a2815414"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn long_voice_text_splits_at_sentences_and_respects_provider_limit() {
        let text = format!("{}. {}.", "a".repeat(350), "b".repeat(300));

        let segments = voice_segments(&text);

        assert_eq!(
            segments,
            vec![
                format!("{}.", "a".repeat(350)),
                format!("{}.", "b".repeat(300))
            ]
        );
        assert!(
            segments
                .iter()
                .all(|part| { part.chars().count() <= MAXIMUM_VOICE_CLIP_CHARACTERS })
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn short_voice_text_is_preserved_byte_for_byte() {
        let text = "  First.\nSecond!  ";

        assert_eq!(voice_segments(text), vec![text]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn role_changes_the_key_of_identical_text() {
        let narrator = VoiceLine {
            role: NARRATOR_ROLE.to_owned(),
            text: "Come closer.".to_owned(),
        };
        let courier = VoiceLine {
            role: "npc:Courier".to_owned(),
            text: "Come closer.".to_owned(),
        };

        assert_ne!(narrator.key(), courier.key());
        assert!(courier.is_npc());
        assert!(!narrator.is_npc());
    }

    #[test]
    fn display_lines_put_narration_before_the_named_speaker() {
        let lines = display_lines(&display("Rain falls.", "Courier", "You are late."));

        assert_eq!(
            lines,
            vec![
                VoiceLine {
                    role: "narrator".to_owned(),
                    text: "Rain falls.".to_owned()
                },
                VoiceLine {
                    role: "npc:Courier".to_owned(),
                    text: "You are late.".to_owned()
                },
            ]
        );
    }

    #[test]
    fn the_rules_note_is_never_voiced() {
        let mut view = display("Rain falls.", "", "");
        view.scene = Some(rpc::GameplayScene {
            rules_note: "Positions are measured in feet.".to_owned(),
            ..Default::default()
        });

        let lines = display_lines(&view);

        assert_eq!(
            lines,
            vec![VoiceLine {
                role: "narrator".to_owned(),
                text: "Rain falls.".to_owned()
            }]
        );
    }

    #[test]
    fn unnamed_dialogue_is_narrated_and_blank_text_is_skipped() {
        let lines = display_lines(&display("  ", "", "A bell tolls."));

        assert_eq!(
            lines,
            vec![VoiceLine {
                role: "narrator".to_owned(),
                text: "A bell tolls.".to_owned()
            }]
        );
    }

    fn replied(last: rpc::TableLine) -> rpc::DisplayGameplayView {
        let mut view = display("Rain falls.", "Courier", "You are late.");
        if let Some(journey) = view.journey.as_mut() {
            journey.table_log = vec![last];
        }
        view
    }

    fn line(
        role: rpc::TableRole,
        kind: rpc::TableKind,
        speaker: &str,
        text: &str,
    ) -> rpc::TableLine {
        rpc::TableLine {
            sequence: 1,
            speaker: speaker.to_owned(),
            role: role as i32,
            hero_index: 0,
            kind: kind as i32,
            text: text.to_owned(),
        }
    }

    #[test]
    fn a_reply_that_is_the_last_thing_said_is_voiced_as_its_speaker() {
        let lines = display_lines(&replied(line(
            rpc::TableRole::Npc,
            rpc::TableKind::Speech,
            "Courier",
            "Keep your voice down.",
        )));

        assert_eq!(
            lines.last(),
            Some(&VoiceLine {
                role: "npc:Courier".to_owned(),
                text: "Keep your voice down.".to_owned()
            })
        );
        assert_eq!(lines.len(), 3);
    }

    fn fight_display() -> rpc::DisplayGameplayView {
        let cry = rpc::FightLine {
            role: rpc::TableRole::Hero as i32,
            hero_index: 0,
            speaker: "Brynn".to_owned(),
            text: "Take that!".to_owned(),
        };
        let closing = rpc::FightLine {
            role: rpc::TableRole::Narrator as i32,
            hero_index: 0,
            speaker: String::new(),
            text: "The fight is won.".to_owned(),
        };
        rpc::DisplayGameplayView {
            journey: Some(rpc::JourneyView {
                combat: Some(rpc::CombatView {
                    outcomes: vec![rpc::CombatOutcome {
                        lines: vec![cry],
                        ..Default::default()
                    }],
                    closing_lines: vec![closing],
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn fight_cries_and_closing_lines_are_voiced_after_the_scene_lines() {
        let lines = display_lines(&fight_display());

        assert_eq!(
            lines,
            vec![
                VoiceLine {
                    role: "npc:Brynn".to_owned(),
                    text: "Take that!".to_owned()
                },
                VoiceLine {
                    role: "narrator".to_owned(),
                    text: "The fight is won.".to_owned()
                },
            ]
        );
    }

    #[test]
    fn fight_lines_without_text_or_speaker_are_not_voiced() {
        assert_eq!(
            fight_voice_line(&rpc::FightLine {
                role: rpc::TableRole::Npc as i32,
                hero_index: 0,
                speaker: String::new(),
                text: "Raaagh!".to_owned(),
            }),
            None
        );
        assert_eq!(
            fight_voice_line(&rpc::FightLine {
                role: rpc::TableRole::Narrator as i32,
                hero_index: 0,
                speaker: String::new(),
                text: "   ".to_owned(),
            }),
            None
        );
    }

    #[test]
    fn a_reply_followed_by_anything_else_is_not_spoken_again() {
        let lines = display_lines(&replied(line(
            rpc::TableRole::Hero,
            rpc::TableKind::Action,
            "Brakka",
            "enters the Lantern Wharf",
        )));
        assert_eq!(lines.len(), 2);
        let lines = display_lines(&replied(line(
            rpc::TableRole::Hero,
            rpc::TableKind::Speech,
            "Brakka",
            "Who sent you?",
        )));
        assert_eq!(lines.len(), 2);
    }
}
