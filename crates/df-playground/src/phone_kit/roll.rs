//! The Roll panel: the check that was called, what is at stake, any advantage, the big Roll button
//! and the result once it is in. The result is the shared screen's `RollView`, worded by the roll
//! card's own helpers (`kit::roll_card`), so a roll reads the same on the phone and the TV.

use serde::{Deserialize, Serialize};

use crate::fixture::{RollMode, RollView};
use crate::kit::roll_card::{RollCard, mode_word};

/// Where the roll stands.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollState {
    /// The check is called; the player has not rolled.
    #[default]
    Ready,
    /// The dice are in the air.
    Rolling,
    Done(RollView),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollPanel {
    /// The check by name: "Perception check".
    pub check: String,
    /// What counts toward it: "Wisdom, proficient: +5".
    #[serde(default)]
    pub basis: Option<String>,
    /// What is at stake, in a line.
    pub stakes: String,
    #[serde(default)]
    pub mode: RollMode,
    /// Why there is an advantage or disadvantage: "You are hidden".
    #[serde(default)]
    pub mode_reason: Option<String>,
    #[serde(default)]
    pub state: RollState,
}

impl RollPanel {
    /// The label of the Roll button now.
    pub fn button_label(&self) -> String {
        match (&self.state, mode_word(self.mode)) {
            (RollState::Ready, None) => "Roll".to_owned(),
            (RollState::Ready, Some(word)) => format!("Roll with {}", word.to_lowercase()),
            (RollState::Rolling, _) => "Rolling...".to_owned(),
            (RollState::Done(_), _) => "Rolled".to_owned(),
        }
    }

    /// Whether the button can be pressed.
    pub fn can_roll(&self) -> bool {
        self.state == RollState::Ready
    }

    /// The pill under the check, or `None` for a normal roll.
    pub fn mode_pill(&self) -> Option<String> {
        let keep = match self.mode {
            RollMode::Normal => return None,
            RollMode::Advantage => "higher",
            RollMode::Disadvantage => "lower",
        };
        mode_word(self.mode).map(|word| format!("{word}: roll two, keep the {keep}"))
    }
}

/// The sum as it is read aloud, from the roll card's own terms: `14 + 3 + 2 = 19`.
pub fn sum_text(view: &RollView) -> String {
    let card = RollCard::from(view);
    let mut text = String::new();
    for (index, term) in card.terms().iter().enumerate() {
        match (index, term.negative) {
            (0, false) => {}
            (0, true) => text.push('-'),
            (_, false) => text.push_str(" + "),
            (_, true) => text.push_str(" - "),
        }
        text.push_str(&term.number);
    }
    format!("{text} = {}", card.total_text())
}

/// Which dice were rolled and which counted, or `None` when every die counted.
pub fn kept_text(view: &RollView) -> Option<String> {
    if view.dice.iter().all(|die| die.kept) {
        return None;
    }
    let faces = |kept: Option<bool>| -> Vec<String> {
        view.dice
            .iter()
            .filter(|die| kept.is_none_or(|wanted| die.kept == wanted))
            .map(|die| die.face.to_string())
            .collect()
    };
    Some(format!(
        "Rolled {}, kept {}",
        faces(None).join(" and "),
        faces(Some(true)).join(" and ")
    ))
}
