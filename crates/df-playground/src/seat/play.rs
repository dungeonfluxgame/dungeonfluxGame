//! The play screen of a seat that plays a character, from its live seat view: the Story page (the
//! place, the lines heard so far, the character's own latest thought) as the phone's scene, and the
//! Act page (what the table asks now, its suggested lines and whether the composer may speak).
//! The picture behind is the shared screen's own plate, which the page already follows.
//!
//! It decides nothing about the story: every line, prompt and suggestion is the host's. A seat
//! that plays no character has no play screen here.

use crate::fixture::{NoteCard, PhoneScene, Status};
use crate::phone_kit::act::ActionCard;
use crate::seat::screen::{JoinCard, SeatTap};
use crate::seat::wire::{DayEnd, SeatView, StoryLine};

/// What the Act page shows.
#[derive(Debug, Clone, PartialEq)]
pub struct SeatAct {
    /// The table's question now, or none while it asks nothing.
    pub prompt: Option<String>,
    /// Why the last answer could not be taken, shown above the cards.
    pub note: Option<String>,
    /// A word from the table for the seat (`SeatView::notice`), shown until its next ask.
    pub notice: Option<String>,
    /// The roll the dungeon master calls for, shown as the shared screen shows it, with Roll.
    pub pending_roll: Option<crate::seat::wire::PendingRoll>,
    /// The seat's last roll, the shared screen's card.
    pub last_roll: Option<crate::fixture::RollView>,
    /// The suggested lines, each sending its line as written.
    pub cards: Vec<JoinCard>,
}

/// The play screen of a seat that plays a character.
#[derive(Debug, Clone, PartialEq)]
pub struct SeatPlay {
    /// The character's name.
    pub hero: String,
    /// The Story page, in the phone scene's shape.
    pub phone: PhoneScene,
    /// What the character saw when they last looked, shown under their private note.
    pub seen: Vec<String>,
    pub act: SeatAct,
}

/// The seat's last roll as the phone first saw it: the roll's id, the ask it saw beside it, and
/// whether the table has asked something else since.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RollSeen {
    roll: Option<String>,
    ask: Option<u64>,
    past: bool,
}

impl RollSeen {
    /// Notes `view`'s last roll, and says whether Act still shows it: from the view it first came
    /// in until the table asks something else. The host keeps the seat's last roll in every view,
    /// so a result from many turns ago would otherwise stay under the cards.
    pub fn still_current(&mut self, view: &SeatView) -> bool {
        let Some(roll) = view.last_roll.as_deref() else {
            *self = Self::default();
            return false;
        };
        let ask = view.ask.as_ref().map(|ask| ask.id);
        if self.roll.as_deref() != Some(roll.id.as_str()) {
            *self = Self {
                roll: Some(roll.id.clone()),
                ask,
                past: false,
            };
        }
        self.past |= self.ask != ask;
        !self.past
    }
}

/// One line of the story as the phone shows it: with the name the host gave its speaker (the
/// hero's own name for the hero's lines), the narrator's as it is.
fn story_line(hero: &str, line: &StoryLine) -> String {
    match (&line.speaker, line.you) {
        (None, true) => format!("{hero}: {}", line.text),
        (Some(speaker), _) => format!("{speaker}: {}", line.text),
        (None, false) => line.text.clone(),
    }
}

/// The play screen of `view`, or `None` when the seat plays no character.
pub fn play_of(view: &SeatView) -> Option<SeatPlay> {
    let story = view.story.as_ref()?;
    let hero = story.hero.clone();
    let phone = PhoneScene {
        hero: hero.clone(),
        goal: story.place.clone(),
        // While the table asks nothing the composer is quiet, and the status says why, from the
        // player's side: the hero acting is their own. Once the day ended the act page says so
        // instead.
        status: if view.ask.is_some() || view.day_end.is_some() {
            Status::Ok
        } else {
            Status::WaitingFor {
                who: "your move".to_owned(),
                what: "to play out".to_owned(),
            }
        },
        narration: story
            .lines
            .iter()
            .map(|line| story_line(&hero, line))
            .collect(),
        suggested: Vec::new(),
        groups: Vec::new(),
        unavailable: Vec::new(),
        note: story.thought.as_ref().map(|thought| NoteCard {
            text: thought.thought.clone(),
        }),
        image: None,
    };
    let act = match &view.ask {
        Some(ask) => SeatAct {
            prompt: Some(ask.prompt.clone()),
            note: ask.note.clone(),
            notice: view.notice.clone(),
            pending_roll: view.pending_roll.as_deref().cloned(),
            last_roll: view.last_roll.as_deref().cloned(),
            cards: ask
                .cards
                .iter()
                .enumerate()
                .map(|(index, card)| JoinCard {
                    picture: None,
                    card: ActionCard {
                        label: card.label.clone(),
                        explain: Some(card.line.clone()),
                        glyph: None,
                        lit: false,
                    },
                    mark: None,
                    tap: SeatTap::Say(index),
                })
                .collect(),
        },
        None => {
            let words = view.day_end.map(DayEnd::player_words);
            SeatAct {
                prompt: words.map(|(said, _)| said.to_owned()),
                note: words.map(|(_, next)| next.to_owned()),
                notice: view.notice.clone(),
                pending_roll: view.pending_roll.as_deref().cloned(),
                last_roll: view.last_roll.as_deref().cloned(),
                cards: Vec::new(),
            }
        }
    };
    let seen = story
        .thought
        .as_ref()
        .map(|thought| thought.saw.clone())
        .unwrap_or_default();
    Some(SeatPlay {
        hero,
        phone,
        seen,
        act,
    })
}
