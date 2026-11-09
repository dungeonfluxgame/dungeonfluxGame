//! The in-game TV UI kit: every panel of the shared screen in one premium-glass look, drawn from
//! the concept boards in `assets/concept-art/` (inventory in `artifacts/tmp/kit/inventory.md`).
//!
//! Each component is a module holding its typed input (serde-ready, every part optional where the
//! boards allow it) and the view decisions made from that input, as plain functions native tests
//! cover. The drawing is `draw` (browser only), one child per component, styled by
//! `assets/playground/kit.css`. Nothing here decides a rule: every value is what the server
//! projected, and an input that names an existing scene type converts from it (`From`), so the old
//! data reaches the kit unchanged.

use serde::{Deserialize, Serialize};

use crate::overlay::Progress;

pub mod banner;
pub mod brand;
pub mod caption_bar;
pub mod choices;
pub mod clue_board;
#[cfg(debug_assertions)]
pub mod gallery;
pub mod journal;
pub mod ledger;
pub mod nameplate;
pub mod narration;
pub mod party;
pub mod place_label;
pub mod progress_line;
pub mod section_label;
pub mod speaker;
pub mod speech;
pub mod status;
pub mod title_card;
pub mod title_chip;

#[cfg(target_arch = "wasm32")]
pub mod draw;

#[cfg(test)]
mod banner_tests;
#[cfg(test)]
mod brand_tests;
#[cfg(test)]
mod caption_bar_tests;
#[cfg(test)]
mod choices_tests;
#[cfg(test)]
mod clue_board_tests;
#[cfg(all(test, debug_assertions))]
mod gallery_tests;
#[cfg(test)]
mod journal_tests;
#[cfg(test)]
mod kit_tests;
#[cfg(test)]
mod ledger_tests;
#[cfg(test)]
mod nameplate_tests;
#[cfg(test)]
mod narration_tests;
#[cfg(test)]
mod party_tests;
#[cfg(test)]
mod place_label_tests;
#[cfg(test)]
mod progress_line_tests;
#[cfg(test)]
mod section_label_tests;
#[cfg(test)]
mod speaker_tests;
#[cfg(test)]
mod speech_tests;
#[cfg(test)]
mod status_tests;
#[cfg(test)]
mod title_card_tests;
#[cfg(test)]
mod title_chip_tests;

/// Every kit panel one screen can show at once. This is the shape a `kit` field of `TvScene`
/// would take (inventory F8); today the gallery boards carry it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KitPanels {
    pub brand: Option<brand::BrandMark>,
    pub chips: Vec<title_chip::TitleChip>,
    pub status: Option<status::StatusChip>,
    pub banners: Vec<banner::Banner>,
    pub progress: Option<progress_line::ProgressLine>,
    pub narration: Option<narration::Narration>,
    pub bubbles: Vec<speech::SpeechBubble>,
    pub places: Vec<place_label::PlaceLabel>,
    pub caption: Option<caption_bar::CaptionBar>,
    pub nameplate: Option<nameplate::Nameplate>,
    pub choices: Option<choices::ChoicePanel>,
    pub title_cards: Vec<title_card::TitleCard>,
    pub speaker: Option<speaker::SpeakerPanel>,
    pub journal: Option<journal::Journal>,
    pub ledgers: Vec<ledger::Ledger>,
    pub clues: Option<clue_board::KitClues>,
    pub section: Option<section_label::SectionLabel>,
    pub party: Option<party::PartyStrip>,
}

/// The most large glass panes that blur the plate at once (inventory F3): the narration box, the
/// party bar, the chips, the banners, the caption, the lists, the clue board and the speaker card.
/// Each samples and filters the plate behind a large area, so they are few.
pub const BLURRED_PANES_AT_MOST: usize = 4;

/// The most small glass panes that blur the plate at once with the fine radius: the bubbles and
/// the thinking cue, the choices and the loose party cards. Each covers a small area that stays
/// still, so its blur is filtered once and kept until the plate behind it moves.
pub const FINE_PANES_AT_MOST: usize = 12;

/// How a glass panel is filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    /// Translucent, blurring the plate behind it: a large pane.
    Blurred,
    /// Translucent, blurring the plate behind it with a small radius: a small pane.
    Fine,
    /// Near-opaque, without a blur.
    Solid,
}

impl KitPanels {
    /// Whether the section label stands under the journal, in one column with the book, rather
    /// than in the bottom stack: the book grows down toward the label, so the two share a column
    /// and the book's height is bounded by the label's (render 36).
    pub fn section_under_journal(&self) -> bool {
        self.section.is_some()
            && self
                .journal
                .as_ref()
                .is_some_and(|journal| journal.is_drawn())
    }

    /// How many drawn small panes blur the plate with the fine radius: each bubble, each choice and
    /// each loose party card.
    pub fn fine_panes(&self) -> usize {
        let bubbles = self
            .bubbles
            .iter()
            .filter(|bubble| bubble.fill() == Fill::Fine)
            .count();
        let choices = self.choices.as_ref().map_or(0, |panel| {
            if panel.fill() == Fill::Fine {
                panel.drawn().len()
            } else {
                0
            }
        });
        let cards = self.party.as_ref().map_or(0, |strip| {
            if strip.fill() == Fill::Fine {
                strip.cards().len()
            } else {
                0
            }
        });
        bubbles + choices + cards
    }

    /// How many drawn panes blur the plate: each component's own `Fill`, counted for the panels
    /// it draws.
    pub fn blurred_panes(&self) -> usize {
        let chips = self
            .chips
            .iter()
            .filter(|chip| chip.title().is_some())
            .count();
        let status = usize::from(
            self.status
                .as_ref()
                .is_some_and(|status| status.text().is_some()),
        );
        let banners = self
            .banners
            .iter()
            .filter(|banner| banner.title().is_some())
            .count();
        let narration = usize::from(self.narration.is_some());
        let caption = usize::from(
            self.caption
                .as_ref()
                .is_some_and(|bar| bar.text().is_some()),
        );
        let speaker = usize::from(self.speaker.is_some());
        let party = usize::from(
            self.party
                .as_ref()
                .is_some_and(|strip| !strip.cards().is_empty() && strip.fill() == Fill::Blurred),
        );
        let ledgers = self
            .ledgers
            .iter()
            .filter(|ledger| ledger.is_drawn() && ledger.fill() == Fill::Blurred)
            .count();
        let clues = usize::from(
            self.clues
                .as_ref()
                .is_some_and(|clues| !clues.notes().is_empty()),
        );
        let places = self
            .places
            .iter()
            .filter(|place| place.text().is_some() && place.fill() == Fill::Blurred)
            .count();
        let cards = self
            .title_cards
            .iter()
            .filter(|card| card.is_drawn() && card.fill() == Fill::Blurred)
            .count();
        chips
            + status
            + banners
            + narration
            + caption
            + speaker
            + party
            + ledgers
            + clues
            + places
            + cards
    }
}

/// The text when it has more than white space, trimmed.
pub fn shown(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|text| !text.is_empty())
}

/// Where one scene of a run stands against the current one: a dot of the title chip's track or a
/// segment of the progress line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Past,
    Current,
    Future,
}

impl Step {
    /// The word that ends the step's class (`kit-step-current`). It is never sent or read.
    pub fn dom_marker(self) -> &'static str {
        match self {
            Self::Past => "past",
            Self::Current => "current",
            Self::Future => "future",
        }
    }
}

/// The scene index of `progress` held inside its run: at least 1, at most the total.
pub fn current_index(progress: &Progress) -> u16 {
    progress.index.clamp(1, progress.total.max(1))
}

/// One step per scene of `progress`; none when the run has fewer than two scenes, since a track
/// of one says nothing.
pub fn steps(progress: &Progress) -> Vec<Step> {
    if progress.total < 2 {
        return Vec::new();
    }
    let current = current_index(progress);
    (1..=progress.total)
        .map(|scene| match scene.cmp(&current) {
            std::cmp::Ordering::Less => Step::Past,
            std::cmp::Ordering::Equal => Step::Current,
            std::cmp::Ordering::Greater => Step::Future,
        })
        .collect()
}

/// The colour family of a glass panel: the warm gold of the boards, or the crimson of a twist and
/// a combat banner (renders 35 and 03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelTone {
    #[default]
    Gold,
    Danger,
}

impl PanelTone {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Gold => "gold",
            Self::Danger => "danger",
        }
    }
}

/// How rare an item is, which tints its name and the rarity line of its title card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    VeryRare,
    Legendary,
}

impl Rarity {
    /// The word that ends the CSS class, the same as the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Common => "common",
            Self::Uncommon => "uncommon",
            Self::Rare => "rare",
            Self::VeryRare => "very_rare",
            Self::Legendary => "legendary",
        }
    }

    /// The word of the rarity line, as the 2024 rules name the rarities.
    pub fn word(self) -> &'static str {
        match self {
            Self::Common => "Common",
            Self::Uncommon => "Uncommon",
            Self::Rare => "Rare",
            Self::VeryRare => "Very Rare",
            Self::Legendary => "Legendary",
        }
    }
}
