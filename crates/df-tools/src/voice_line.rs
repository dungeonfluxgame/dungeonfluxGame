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

/// The public result of the most recent non-combat check, assembled from the server's table
/// log.  The sequence identifies the check's roll line so the browser can queue this exactly
/// once while the result card is being presented.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CheckVoiceLine {
    pub(crate) sequence: u64,
    pub(crate) result_sequence: u64,
    pub(crate) finding: String,
    pub(crate) voice: VoiceLine,
}

/// A missing cast identity permits legacy name routing; an ambiguous name permits none.
pub(crate) enum SceneSpeaker<'a> {
    Matched(&'a rpc::SceneActor),
    Missing,
    Ambiguous,
}

/// Human speaker names ignore case; canonical actor IDs and spoken text do not.
pub(crate) fn speaker_names_match(left: &str, right: &str) -> bool {
    left.trim().to_lowercase() == right.trim().to_lowercase()
}

/// Resolve a public name once, retaining the cast's canonical spelling and exact actor ID.
pub(crate) fn scene_speaker<'a>(
    scene: Option<&'a rpc::GameplayScene>,
    name: &str,
) -> SceneSpeaker<'a> {
    let Some(scene) = scene else {
        return SceneSpeaker::Missing;
    };
    let mut matches = scene
        .cast
        .iter()
        .filter(|actor| speaker_names_match(&actor.name, name));
    let Some(actor) = matches.next() else {
        return SceneSpeaker::Missing;
    };
    if matches.any(|other| other.id != actor.id) {
        SceneSpeaker::Ambiguous
    } else {
        SceneSpeaker::Matched(actor)
    }
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

    pub(crate) fn actor_id(&self) -> Option<&str> {
        self.role
            .strip_prefix("npc-id:")
            .and_then(|value| value.split_once('|').map(|(id, _)| id))
    }

    /// The speaker name of a named-speaker line; `None` for the narrator.
    pub(crate) fn speaker(&self) -> Option<&str> {
        self.role
            .strip_prefix("npc-id:")
            .and_then(|value| value.split_once('|').map(|(_, name)| name))
            .or_else(|| self.role.strip_prefix(NPC_ROLE_PREFIX))
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
    let mut lines = Vec::with_capacity(3);
    if !display.narration.trim().is_empty() {
        lines.push(VoiceLine {
            role: NARRATOR_ROLE.to_owned(),
            text: display.narration.clone(),
        });
    }
    if let Some(journey) = display.journey.as_ref()
        && !journey.dialogue.trim().is_empty()
    {
        let speaker = journey.speaker.trim();
        let role = if speaker.is_empty() {
            Some(NARRATOR_ROLE.to_owned())
        } else {
            match scene_speaker(display.scene.as_ref(), speaker) {
                SceneSpeaker::Matched(actor) if !actor.id.is_empty() => {
                    Some(format!("npc-id:{}|{}", actor.id, actor.name))
                }
                SceneSpeaker::Matched(actor) => Some(format!("{NPC_ROLE_PREFIX}{}", actor.name)),
                SceneSpeaker::Missing => Some(format!("{NPC_ROLE_PREFIX}{speaker}")),
                SceneSpeaker::Ambiguous => None,
            }
        };
        if let Some(role) = role {
            lines.push(VoiceLine {
                role,
                text: journey.dialogue.clone(),
            });
        }
    }
    if let Some(check) = check_voice_line(display) {
        lines.push(check.voice);
    }
    lines.extend(scene_outcomes(display));
    for line in conversation_replies(display) {
        if !lines
            .iter()
            .any(|known| known.role == line.role && known.text == line.text)
        {
            lines.push(line);
        }
    }
    lines.extend(fight_lines(display));
    lines
}

/// Committed public interaction outcomes use the narrator's voice. Check rows already have
/// their own result line, and combat outcomes keep their existing per-beat script.
fn scene_outcomes(display: &rpc::DisplayGameplayView) -> Vec<VoiceLine> {
    let Some(journey) = display.journey.as_ref() else {
        return Vec::new();
    };
    if journey.combat.is_some() {
        return Vec::new();
    }
    let check = check_voice_line(display);
    journey
        .table_log
        .iter()
        .filter(|line| {
            rpc::TableRole::try_from(line.role) == Ok(rpc::TableRole::Narrator)
                && rpc::TableKind::try_from(line.kind) == Ok(rpc::TableKind::Action)
                && line.speaker.trim() != "Roll"
                && !line.text.trim().is_empty()
                && check
                    .as_ref()
                    .is_none_or(|check| line.sequence != check.result_sequence)
        })
        .map(|line| VoiceLine {
            role: NARRATOR_ROLE.to_owned(),
            text: line.text.trim().to_owned(),
        })
        .collect()
}

/// Pair the public roll with its immediately following finding row. The card displays the
/// finding; the DM voice announces only the public outcome and that same finding.
pub(crate) fn check_voice_line(display: &rpc::DisplayGameplayView) -> Option<CheckVoiceLine> {
    display.check.as_ref()?;
    let journey = display.journey.as_ref()?;
    let roll = journey.table_log.iter().rev().find(|line| {
        rpc::TableRole::try_from(line.role) == Ok(rpc::TableRole::Narrator)
            && rpc::TableKind::try_from(line.kind) == Ok(rpc::TableKind::Action)
            && line.speaker.trim() == "Roll"
            && !line.text.trim().is_empty()
    })?;
    let result = journey.table_log.iter().find(|line| {
        line.sequence == roll.sequence.saturating_add(1)
            && rpc::TableRole::try_from(line.role) == Ok(rpc::TableRole::Narrator)
            && rpc::TableKind::try_from(line.kind) == Ok(rpc::TableKind::Action)
            && line.speaker.trim() != "Roll"
            && !line.text.trim().is_empty()
    })?;
    Some(CheckVoiceLine {
        sequence: roll.sequence,
        result_sequence: result.sequence,
        finding: result.text.trim().to_owned(),
        voice: VoiceLine {
            role: NARRATOR_ROLE.to_owned(),
            text: check_voice_text(display.check.as_ref()?, result.text.trim()),
        },
    })
}

fn check_voice_text(check: &rpc::AbilityCheckResult, finding: &str) -> String {
    let finding = finding.trim();
    if !check.succeeded {
        return if finding.is_empty() {
            "The attempt was unsuccessful.".to_owned()
        } else {
            finding.to_owned()
        };
    }
    if finding.is_empty() {
        return "Success.".to_owned();
    }
    let finding = if finding
        .chars()
        .next()
        .is_some_and(|character| character.is_lowercase())
    {
        let fragment = finding.trim_end_matches(['.', '!', '?']).trim_end();
        format!("You notice {fragment}.")
    } else if finding.ends_with(['.', '!', '?']) {
        finding.to_owned()
    } else {
        format!("You notice {finding}.")
    };
    format!("Success. {finding}")
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

/// Every permitted public NPC speech row in the table log, in log order. The display view
/// is already audience-filtered, so an explicit actor id and public speaker on a row are
/// authority enough: no figure or plate cast entry is required. A cast entry for that id
/// supplies its canonical name when the public name matches it; a contradictory name for
/// a known id is refused. An empty id keeps the unambiguous legacy name path. Duplicate
/// role/text keys are kept once.
fn conversation_replies(display: &rpc::DisplayGameplayView) -> Vec<VoiceLine> {
    let Some(journey) = display.journey.as_ref() else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for row in &journey.table_log {
        let spoken_by_npc = rpc::TableRole::try_from(row.role) == Ok(rpc::TableRole::Npc)
            && rpc::TableKind::try_from(row.kind) == Ok(rpc::TableKind::Speech);
        let (speaker, text) = (row.speaker.trim(), row.text.trim());
        if !spoken_by_npc || speaker.is_empty() || text.is_empty() {
            continue;
        }
        let line = if row.actor_id.is_empty() {
            let speaker = match scene_speaker(display.scene.as_ref(), speaker) {
                SceneSpeaker::Matched(actor) => actor.name.as_str(),
                SceneSpeaker::Missing => speaker,
                SceneSpeaker::Ambiguous => continue,
            };
            VoiceLine {
                role: format!("{NPC_ROLE_PREFIX}{speaker}"),
                text: text.to_owned(),
            }
        } else {
            let known = display
                .scene
                .as_ref()
                .and_then(|scene| scene.cast.iter().find(|actor| actor.id == row.actor_id));
            let speaker = match known {
                Some(actor) if speaker_names_match(&actor.name, speaker) => actor.name.as_str(),
                Some(_) => continue,
                None => speaker,
            };
            VoiceLine {
                role: format!("npc-id:{}|{speaker}", row.actor_id),
                text: text.to_owned(),
            }
        };
        if !lines
            .iter()
            .any(|known: &VoiceLine| known.role == line.role && known.text == line.text)
        {
            lines.push(line);
        }
    }
    lines
}

/// The newest conversation reply, for callers that only need the latest line.
#[cfg(test)]
fn conversation_reply(display: &rpc::DisplayGameplayView) -> Option<VoiceLine> {
    conversation_replies(display).into_iter().last()
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

    fn courier_display(speaker: &str) -> rpc::DisplayGameplayView {
        let mut shown = display(
            "",
            speaker,
            "The crest belongs to a ship that never appears on the ledger. I think someone aboard followed me.",
        );
        shown.scene = Some(rpc::GameplayScene {
            title: "The Broken Seal".into(),
            cast: vec![rpc::SceneActor {
                id: "courier".into(),
                name: "Courier".into(),
                speaking: true,
                ..Default::default()
            }],
            ..Default::default()
        });
        shown
    }

    #[test]
    fn scripted_speaker_case_preserves_canonical_caption_voice_and_sequence() {
        let mut identity = None;
        for speaker in ["Courier", "courier", "COURIER"] {
            let mut shown = courier_display(speaker);
            let voice = display_lines(&shown).pop().unwrap();
            let view = rpc::ViewMessage {
                audience: Some(rpc::view_message::Audience::Display(shown.clone())),
                ..Default::default()
            };
            let caption = crate::presentation::table_lines(&view).pop().unwrap();
            assert_eq!(caption.actor_id, "courier");
            assert_eq!(caption.speaker, "Courier");
            assert_eq!(voice.actor_id(), Some(caption.actor_id.as_str()));
            assert_eq!(voice.speaker(), Some(caption.speaker.as_str()));
            assert_eq!(voice.text, caption.text);
            let current = (caption.sequence, voice.key());
            assert_eq!(identity.get_or_insert_with(|| current.clone()), &current);

            shown
                .journey
                .as_mut()
                .unwrap()
                .table_log
                .push(rpc::TableLine {
                    actor_id: "courier".into(),
                    speaker: speaker.into(),
                    text: voice.text.clone(),
                    role: rpc::TableRole::Npc as i32,
                    kind: rpc::TableKind::Speech as i32,
                    ..Default::default()
                });
            assert_eq!(conversation_reply(&shown).unwrap().key(), voice.key());
        }
    }

    #[test]
    fn scripted_speaker_without_canonical_id_keeps_legacy_name_voice() {
        let mut shown = courier_display("courier");
        shown.scene.as_mut().unwrap().cast[0].id.clear();
        let voice = display_lines(&shown).pop().unwrap();
        assert_eq!(voice.role, "npc:Courier");
        assert_eq!(voice.actor_id(), None);

        shown.scene.as_mut().unwrap().cast.clear();
        assert_eq!(display_lines(&shown).pop().unwrap().role, "npc:courier");
        shown.scene = None;
        assert_eq!(display_lines(&shown).pop().unwrap().role, "npc:courier");
    }

    #[test]
    fn ambiguous_folded_speaker_names_are_not_voiced_or_projected() {
        let mut shown = courier_display("COURIER");
        shown.scene.as_mut().unwrap().cast.push(rpc::SceneActor {
            id: "other-courier".into(),
            name: "courier".into(),
            ..Default::default()
        });
        shown.journey.as_mut().unwrap().reply_speaker = "courier".into();
        assert!(display_lines(&shown).is_empty());
        let view = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(shown)),
            ..Default::default()
        };
        assert!(crate::presentation::table_lines(&view).is_empty());
        assert!(crate::presentation::replying_speaker(&view).is_none());
    }

    #[test]
    fn canonical_reply_rejects_a_contradictory_name_for_a_known_actor_id() {
        let mut shown = courier_display("Courier");
        shown
            .journey
            .as_mut()
            .unwrap()
            .table_log
            .push(rpc::TableLine {
                actor_id: "courier".into(),
                speaker: "Bandit".into(),
                text: "Give me the packet.".into(),
                role: rpc::TableRole::Npc as i32,
                kind: rpc::TableKind::Speech as i32,
                ..Default::default()
            });
        assert!(conversation_reply(&shown).is_none());
    }

    #[test]
    fn canonical_reply_accepts_an_explicit_actor_id_absent_from_the_cast() {
        for (actor_id, speaker) in [("departed", "courier"), ("COURIER", "Courier")] {
            let mut shown = courier_display("Courier");
            shown
                .journey
                .as_mut()
                .unwrap()
                .table_log
                .push(rpc::TableLine {
                    actor_id: actor_id.into(),
                    speaker: speaker.into(),
                    text: "Give me the packet.".into(),
                    role: rpc::TableRole::Npc as i32,
                    kind: rpc::TableKind::Speech as i32,
                    ..Default::default()
                });
            let reply = conversation_reply(&shown).unwrap();
            assert_eq!(reply.actor_id(), Some(actor_id), "{actor_id}/{speaker}");
            assert_eq!(reply.speaker(), Some(speaker), "{actor_id}/{speaker}");
        }
    }

    #[test]
    fn public_npc_speech_without_a_cast_figure_registers_every_reply_once() {
        let mut shown = display("", "", "");
        shown.scene = Some(rpc::GameplayScene {
            title: "Salt Quay".into(),
            cast: vec![rpc::SceneActor {
                id: "courier".into(),
                name: "Courier".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        let row = |sequence: u64, text: &str| rpc::TableLine {
            sequence,
            actor_id: "bandit".into(),
            speaker: "Dain Rusk".into(),
            text: text.into(),
            role: rpc::TableRole::Npc as i32,
            kind: rpc::TableKind::Speech as i32,
            ..Default::default()
        };
        shown.journey.as_mut().unwrap().table_log = vec![
            row(1, "The packet stays with me."),
            row(2, "Ask the harbourmaster."),
            row(3, "The packet stays with me."),
        ];
        let expected = vec![
            VoiceLine {
                role: "npc-id:bandit|Dain Rusk".into(),
                text: "The packet stays with me.".into(),
            },
            VoiceLine {
                role: "npc-id:bandit|Dain Rusk".into(),
                text: "Ask the harbourmaster.".into(),
            },
        ];
        assert_eq!(conversation_replies(&shown), expected);
        assert_eq!(display_lines(&shown), expected);
        assert_eq!(
            conversation_reply(&shown).unwrap().text,
            "Ask the harbourmaster."
        );
    }

    #[test]
    fn pending_speaker_name_and_answer_match_ignore_case() {
        let mut shown = courier_display("courier");
        shown.journey.as_mut().unwrap().reply_speaker = "COURIER".into();
        let mut view = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(shown)),
            ..Default::default()
        };
        assert_eq!(
            crate::presentation::replying_speaker(&view).as_deref(),
            Some("Courier")
        );
        let Some(rpc::view_message::Audience::Display(shown)) = view.audience.as_mut() else {
            panic!("display view");
        };
        let journey = shown.journey.as_mut().unwrap();
        journey.table_log.push(rpc::TableLine {
            sequence: 1,
            actor_id: "courier".into(),
            speaker: "cOuRiEr".into(),
            text: "Stay close.".into(),
            role: rpc::TableRole::Npc as i32,
            kind: rpc::TableKind::Speech as i32,
            ..Default::default()
        });
        assert_eq!(
            crate::presentation::newest_scene_speech(journey).unwrap().2,
            "Stay close."
        );
        assert!(crate::presentation::replying_speaker(&view).is_none());
    }

    #[test]
    fn canonical_actor_identity_controls_reply_voice_and_cache_key() {
        let mut display = display("", "Courier", "");
        display.scene = Some(rpc::GameplayScene {
            cast: vec![rpc::SceneActor {
                id: "bandit".into(),
                name: "Bandit".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        display
            .journey
            .as_mut()
            .unwrap()
            .table_log
            .push(rpc::TableLine {
                actor_id: "bandit".into(),
                speaker: "Bandit".into(),
                role: rpc::TableRole::Npc as i32,
                kind: rpc::TableKind::Speech as i32,
                text: "Give me the packet.".into(),
                ..Default::default()
            });
        let reply = conversation_reply(&display).unwrap();
        assert_eq!(reply.actor_id(), Some("bandit"));
        assert_eq!(reply.speaker(), Some("Bandit"));
        assert_ne!(
            reply.key(),
            VoiceLine {
                role: "npc:Courier".into(),
                text: reply.text.clone()
            }
            .key()
        );
        display
            .journey
            .as_mut()
            .unwrap()
            .table_log
            .last_mut()
            .unwrap()
            .actor_id = "courier".into();
        let reassigned = conversation_reply(&display).unwrap();
        assert_eq!(reassigned.actor_id(), Some("courier"));
        assert_eq!(reassigned.speaker(), Some("Bandit"));
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

    #[test]
    fn check_voice_line_announces_outcome_and_authoritative_finding_only() {
        let mut view = display("", "", "");
        view.check = Some(rpc::AbilityCheckResult {
            total: 8,
            succeeded: true,
            ..Default::default()
        });
        view.journey.as_mut().unwrap().table_log = vec![
            rpc::TableLine {
                sequence: 8,
                role: rpc::TableRole::Narrator as i32,
                kind: rpc::TableKind::Action as i32,
                speaker: "Roll".into(),
                text: "Brakka searches the packet: Wisdom (Perception) 8 + 0 = 8.".into(),
                ..Default::default()
            },
            rpc::TableLine {
                sequence: 9,
                role: rpc::TableRole::Narrator as i32,
                kind: rpc::TableKind::Action as i32,
                text: "You find nothing unusual.".into(),
                ..Default::default()
            },
        ];

        let check = check_voice_line(&view).expect("public check result");

        assert_eq!(check.sequence, 8);
        assert_eq!(check.result_sequence, 9);
        assert_eq!(check.finding, "You find nothing unusual.");
        assert_eq!(check.voice.role, NARRATOR_ROLE);
        assert_eq!(check.voice.text, "Success. You find nothing unusual.");
        assert!(check.voice.text.contains("You find nothing unusual."));
        assert!(!check.voice.text.contains("Brakka"));
        assert!(!check.voice.text.contains("Wisdom"));
        assert!(!check.voice.text.contains("14"));
        assert!(!check.voice.text.contains("DC"));
        assert_eq!(display_lines(&view), [check.voice]);
    }

    #[test]
    fn check_voice_line_uses_public_failure_without_finding_text() {
        let mut view = display("", "", "");
        view.check = Some(rpc::AbilityCheckResult {
            total: 5,
            succeeded: false,
            ..Default::default()
        });
        view.journey.as_mut().unwrap().table_log = vec![
            line_with_sequence(20, "Roll", "Wisdom (Perception) 5 + 0 = 5."),
            line_with_sequence(21, "", "The seal remains shut."),
        ];

        let check = check_voice_line(&view).expect("public failed check result");

        assert_eq!(check.voice.text, "The seal remains shut.");
        assert!(!check.voice.text.contains("5"));
    }

    #[test]
    fn check_voice_line_requires_a_public_result_after_the_roll() {
        let mut view = display("", "", "");
        view.check = Some(rpc::AbilityCheckResult {
            total: 14,
            succeeded: true,
            ..Default::default()
        });
        view.journey.as_mut().unwrap().table_log = vec![rpc::TableLine {
            sequence: 8,
            role: rpc::TableRole::Narrator as i32,
            kind: rpc::TableKind::Action as i32,
            speaker: "Roll".into(),
            text: "Wisdom (Perception) 1 + 0 = 1.".into(),
            ..Default::default()
        }];

        assert!(check_voice_line(&view).is_none());

        view.journey
            .as_mut()
            .unwrap()
            .table_log
            .push(rpc::TableLine {
                sequence: 10,
                role: rpc::TableRole::Narrator as i32,
                kind: rpc::TableKind::Action as i32,
                text: "A later unrelated narration.".into(),
                ..Default::default()
            });
        assert!(check_voice_line(&view).is_none());
    }

    #[test]
    fn check_voice_line_selects_the_latest_complete_check_pair() {
        let mut view = display("", "", "");
        view.check = Some(rpc::AbilityCheckResult {
            total: 14,
            succeeded: true,
            ..Default::default()
        });
        view.journey.as_mut().unwrap().table_log = vec![
            line_with_sequence(8, "Roll", "Wisdom (Perception) 8 + 0 = 8."),
            line_with_sequence(9, "", "The packet is ordinary."),
            line_with_sequence(10, "Roll", "Wisdom (Perception) 14 + 0 = 14."),
            line_with_sequence(11, "", "The seal is warm to the touch."),
        ];

        let check = check_voice_line(&view).expect("latest complete public check");

        assert_eq!(check.sequence, 10);
        assert_eq!(check.result_sequence, 11);
        assert_eq!(check.finding, "The seal is warm to the touch.");
        assert_eq!(check.voice.text, "Success. The seal is warm to the touch.");
        assert!(!check.voice.text.contains("14"));
    }

    fn line_with_sequence(sequence: u64, speaker: &str, text: &str) -> rpc::TableLine {
        rpc::TableLine {
            sequence,
            role: rpc::TableRole::Narrator as i32,
            kind: rpc::TableKind::Action as i32,
            speaker: speaker.to_owned(),
            text: text.to_owned(),
            ..Default::default()
        }
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
            roll: None,
            actor_id: String::new(),
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
