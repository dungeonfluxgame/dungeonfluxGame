//! The Act panel: what is happening, the suggested actions as cards, the groups of other things to
//! do, and the composer that takes a typed or spoken line.
//!
//! The choice rules (at most four suggested, fixed groups, unavailable choices with a reason) are
//! `choices`; this module only puts the result in the shape the drawing takes.

use serde::{Deserialize, Serialize};

use crate::choices::ChoiceListView;
use crate::composer::{ComposerInputs, ComposerState, ComposerView, SpeechAvailability, Target};
use crate::fixture::Choice;
use crate::overlay::Glyph;

/// One action the player may take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionCard {
    pub label: String,
    #[serde(default)]
    pub explain: Option<String>,
    #[serde(default)]
    pub glyph: Option<Glyph>,
    /// The card the player has picked or the host leans to, drawn lit.
    #[serde(default)]
    pub lit: bool,
}

impl From<&Choice> for ActionCard {
    fn from(choice: &Choice) -> Self {
        Self {
            label: choice.label.clone(),
            explain: choice.explain.clone(),
            glyph: None,
            lit: false,
        }
    }
}

/// A fixed group of smaller choices (Attack, Move, Talk and so on).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionGroup {
    pub name: String,
    pub choices: Vec<ActionCard>,
}

/// An action that cannot be taken now, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OffCard {
    pub label: String,
    pub reason: String,
}

/// What the player may do.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ActChoices {
    /// The suggested cards the choice-list rule keeps (`choices::build`).
    pub suggested: Vec<ActionCard>,
    /// How many more suggestions the rule left out.
    pub left_out: usize,
    pub groups: Vec<ActionGroup>,
    pub unavailable: Vec<OffCard>,
}

impl ActChoices {
    /// Whether there is nothing to choose from or to explain.
    pub fn is_empty(&self) -> bool {
        self.suggested.is_empty() && self.groups.is_empty() && self.unavailable.is_empty()
    }
}

impl From<&ChoiceListView<'_>> for ActChoices {
    fn from(view: &ChoiceListView<'_>) -> Self {
        Self {
            suggested: view.suggested.iter().map(ActionCard::from).collect(),
            left_out: view.left_out,
            groups: view
                .groups
                .iter()
                .map(|group| ActionGroup {
                    name: group.name.clone(),
                    choices: group.choices.iter().map(ActionCard::from).collect(),
                })
                .collect(),
            unavailable: view
                .unavailable
                .iter()
                .map(|choice| OffCard {
                    label: choice.label.clone(),
                    reason: choice.reason.clone(),
                })
                .collect(),
        }
    }
}

/// The panel: the story as it stands, and what to do about it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ActPanel {
    pub narration: Vec<String>,
    pub choices: ActChoices,
    /// A line only this player sees.
    pub private_note: Option<String>,
}

/// How the composer looks in a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComposerLook {
    /// Nothing typed or said.
    #[default]
    Idle,
    /// The microphone is on.
    Listening,
    /// What was said is written in the box, to read, change and send.
    Transcript,
}

/// A composer drawn with sample words, built the way the live one is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComposerSample {
    pub look: ComposerLook,
    /// The words in the box for a transcript.
    #[serde(default)]
    pub text: String,
    /// Who the line goes to: names, never ids.
    #[serde(default)]
    pub people: Vec<String>,
}

impl ComposerSample {
    /// What the live composer would show for this sample.
    pub fn view(&self) -> ComposerView {
        let targets: Vec<Target> = self
            .people
            .iter()
            .map(|name| Target {
                id: name.clone(),
                name: name.clone(),
            })
            .collect();
        let state = match self.look {
            ComposerLook::Idle => ComposerState::Idle,
            ComposerLook::Listening => ComposerState::Recording,
            ComposerLook::Transcript => ComposerState::Draft,
        };
        ComposerView::of(&ComposerInputs {
            state,
            has_text: !self.text.trim().is_empty(),
            blocked: false,
            speech: &SpeechAvailability::Ready,
            targets: &targets,
            chosen: None,
            elapsed_ms: 4_000,
            level_permille: 620,
            note: None,
        })
    }
}
