//! Which text of a display view is spoken, and the identity of one spoken line.
//!
//! The server registers these lines when it sends a display view and the browser asks for
//! them by key, so both sides use this one definition and cannot drift apart.
use df_protocol::common as rpc;
use sha2::{Digest, Sha256};

/// Role of the scene narration, and of a dialogue line that has no named speaker.
pub(crate) const NARRATOR_ROLE: &str = "narrator";
const NPC_ROLE_PREFIX: &str = "npc:";

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

    /// Whether the line belongs to a named speaker rather than the narrator.
    pub(crate) fn is_npc(&self) -> bool {
        self.role.starts_with(NPC_ROLE_PREFIX)
    }
}

/// The scene narration first, then the current speaker line; empty text is not spoken.
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
    lines
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
}
