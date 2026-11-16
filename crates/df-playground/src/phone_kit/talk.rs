//! The Talk panel: who is here, whom the hero is speaking to, and the conversation so far as a
//! list of speaker and line. It is a transcript, not speech bubbles over a picture.

use serde::{Deserialize, Serialize};

use crate::lobby::model::BubbleTone;

/// Someone in the scene the hero may speak to. The name is the one the hero has learned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CastMember {
    pub name: String,
    #[serde(default)]
    pub portrait: Option<String>,
    /// What the hero knows of them: "innkeeper", "a stranger in grey".
    #[serde(default)]
    pub known_as: Option<String>,
}

/// One line of the conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadLine {
    pub speaker: String,
    #[serde(default)]
    pub voice: BubbleTone,
    pub text: String,
}

/// A run of lines by one speaker, so the name stands once above them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn<'a> {
    pub speaker: &'a str,
    pub voice: BubbleTone,
    pub lines: Vec<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TalkPanel {
    pub cast: Vec<CastMember>,
    /// The name of the one addressed; none means the whole room.
    pub addressing: Option<String>,
    pub thread: Vec<ThreadLine>,
}

impl TalkPanel {
    /// Who the hero is speaking to, in words.
    pub fn addressing_text(&self) -> String {
        match &self.addressing {
            Some(name) => format!("Speaking to {name}"),
            None => "Speaking to the room".to_owned(),
        }
    }

    /// Whether `name` is the one addressed.
    pub fn is_addressed(&self, name: &str) -> bool {
        self.addressing.as_deref() == Some(name)
    }

    /// The thread as turns: neighbouring lines of one speaker are one turn.
    pub fn turns(&self) -> Vec<Turn<'_>> {
        let mut turns: Vec<Turn<'_>> = Vec::new();
        for line in &self.thread {
            match turns.last_mut() {
                Some(turn) if turn.speaker == line.speaker => turn.lines.push(&line.text),
                _ => turns.push(Turn {
                    speaker: &line.speaker,
                    voice: line.voice,
                    lines: vec![&line.text],
                }),
            }
        }
        turns
    }
}
