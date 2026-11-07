//! The requests and answers of the state routes (`/dev/persona/state`) and the text the dev
//! command line prints for a state.

use std::fmt::Write;

use df_content::persona::{Attitude, PersonaState};
use serde::{Deserialize, Serialize};

/// The persona a state request is about: the query of `GET /dev/persona/state` and the body of
/// `POST /dev/persona/state/reset`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateTarget {
    pub id: String,
}

/// The body of `POST /dev/persona/state/rapport`: a hand-set standing toward another character
/// (-100 to 100; the attitude follows), for a scene that starts mid-story.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetRapportRequest {
    pub id: String,
    pub toward: String,
    pub rapport: i8,
}

/// The body of `POST /dev/persona/state/set`: a hand-set level, a developer's override.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetEmotionRequest {
    pub id: String,
    pub emotion: String,
    pub level: u8,
}

/// The body of `POST /dev/persona/temperament`: have the text service write the temperament and
/// emotional triggers of the stored persona `id`; everything else of it is untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemperamentRequest {
    pub id: String,
    /// The model to ask instead of the persona default.
    #[serde(default)]
    pub model: Option<String>,
}

/// The live state of a persona and the ids of its emotional triggers whose numeric condition
/// holds in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateView {
    pub state: PersonaState,
    pub holding: Vec<String>,
}

fn attitude_word(attitude: Attitude) -> &'static str {
    match attitude {
        Attitude::Friendly => "friendly",
        Attitude::Indifferent => "indifferent",
        Attitude::Hostile => "hostile",
    }
}

/// The text of `view`: the mood, the emotions above zero, the attention and the standings.
pub(super) fn format_state(view: &StateView) -> String {
    let state = &view.state;
    let mut text = format!(
        "ok state {} turn={} mood valence={} arousal={}",
        state.id, state.turn, state.mood.valence, state.mood.arousal
    );
    let felt: Vec<String> = state
        .emotions
        .iter()
        .filter(|seen| seen.level > 0)
        .map(|seen| format!("{}={}", seen.emotion, seen.level))
        .collect();
    let _ = write!(
        text,
        "\n  emotions: {}",
        if felt.is_empty() {
            "none".to_owned()
        } else {
            felt.join(" ")
        }
    );
    let attention = &state.attention;
    if attention.focus.is_some() || attention.topic.is_some() {
        let _ = write!(
            text,
            "\n  attention: focus={} topic={}",
            attention.focus.as_deref().unwrap_or("-"),
            attention.topic.as_deref().unwrap_or("-")
        );
    }
    for line in &attention.noticed {
        let _ = write!(text, "\n  noticed {}: {}", line.speaker, line.gist);
    }
    for standing in &state.relationships {
        let _ = write!(
            text,
            "\n  toward {}: {} (rapport {})",
            standing.toward,
            attitude_word(standing.attitude),
            standing.rapport
        );
    }
    if !view.holding.is_empty() {
        let _ = write!(text, "\n  triggers holding: {}", view.holding.join(" "));
    }
    text
}
