//! The offers of one pending resolution and who has answered which (C.4).
//!
//! It owns `ResolutionWindow`: one `SeatOffer` per seat that may answer, the choices that seat is
//! shown, and the answer it gave. A window is complete when every offer is answered (a reaction
//! window, a roller's choice) or when any one is (a ruling: the host's answer or the requester's
//! cancel settles it). It never decides what an answer means and never times out: a window holds
//! until it is answered (RES26, `window_timeout` none). Answering returns a changed copy and
//! leaves the window as it was, so a caller that fails afterwards has changed nothing.
//!
//! It fails with `WindowError` when built from offers nobody can answer, and with
//! `AnswerRefusal` when an answer is not allowed: the seat has no offer, the choice is not one of
//! its choices, or the seat already answered.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::identity::SeatId;
use crate::ports::{ActionId, Offer};

/// One seat's share of a window.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatOffer {
    seat: SeatId,
    choices: Vec<ActionId>,
    answer: Option<ActionId>,
}

impl SeatOffer {
    pub fn seat(&self) -> SeatId {
        self.seat
    }

    pub fn choices(&self) -> &[ActionId] {
        &self.choices
    }

    pub fn answer(&self) -> Option<&ActionId> {
        self.answer.as_ref()
    }
}

/// When a window is done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Completion {
    /// Every seat with an offer has answered (a reaction window, the roller's own choice).
    EveryOffer,
    /// One answer settles it (a ruling, a confirmation).
    AnyOffer,
}

/// Why a window could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowError {
    NoOffers,
    SeatOfferedTwice(SeatId),
    NothingToChoose(SeatId),
}

impl fmt::Display for WindowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoOffers => formatter.write_str("a window needs at least one offer"),
            Self::SeatOfferedTwice(seat) => write!(formatter, "seat {seat} has two offers"),
            Self::NothingToChoose(seat) => write!(formatter, "the offer to seat {seat} is empty"),
        }
    }
}

impl std::error::Error for WindowError {}

/// Why an answer was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerRefusal {
    NotEligible,
    NotOffered,
    AlreadyAnswered,
}

/// The offers of a pending resolution.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionWindow {
    offers: Vec<SeatOffer>,
    completion: Completion,
}

impl ResolutionWindow {
    /// A window over these offers, in the order the answers are given to the rules owner.
    pub fn new(offers: Vec<Offer>, completion: Completion) -> Result<Self, WindowError> {
        if offers.is_empty() {
            return Err(WindowError::NoOffers);
        }
        let mut seen = BTreeSet::new();
        let mut built = Vec::with_capacity(offers.len());
        for offer in offers {
            if !seen.insert(offer.seat) {
                return Err(WindowError::SeatOfferedTwice(offer.seat));
            }
            if offer.choices.is_empty() {
                return Err(WindowError::NothingToChoose(offer.seat));
            }
            built.push(SeatOffer {
                seat: offer.seat,
                choices: offer.choices,
                answer: None,
            });
        }
        Ok(Self {
            offers: built,
            completion,
        })
    }

    pub fn offers(&self) -> &[SeatOffer] {
        &self.offers
    }

    pub fn completion(&self) -> Completion {
        self.completion
    }

    pub fn offer_for(&self, seat: SeatId) -> Option<&SeatOffer> {
        self.offers.iter().find(|offer| offer.seat == seat)
    }

    /// A copy with the seat's answer taken, or why it is not.
    pub fn answered(&self, seat: SeatId, choice: &ActionId) -> Result<Self, AnswerRefusal> {
        let offer = self.offer_for(seat).ok_or(AnswerRefusal::NotEligible)?;
        if offer.answer.is_some() {
            return Err(AnswerRefusal::AlreadyAnswered);
        }
        if !offer.choices.contains(choice) {
            return Err(AnswerRefusal::NotOffered);
        }
        let mut next = self.clone();
        for held in &mut next.offers {
            if held.seat == seat {
                held.answer = Some(choice.clone());
            }
        }
        Ok(next)
    }

    /// A copy with the seat's answer taken back, so the seat is asked again.
    pub fn reasked(&self, seat: SeatId) -> Self {
        let mut next = self.clone();
        for held in &mut next.offers {
            if held.seat == seat {
                held.answer = None;
            }
        }
        next
    }

    pub fn is_complete(&self) -> bool {
        match self.completion {
            Completion::EveryOffer => self.offers.iter().all(|offer| offer.answer.is_some()),
            Completion::AnyOffer => self.offers.iter().any(|offer| offer.answer.is_some()),
        }
    }

    /// The answers given, in the order of the offers.
    pub fn answers(&self) -> impl Iterator<Item = (SeatId, &ActionId)> {
        self.offers
            .iter()
            .filter_map(|offer| offer.answer.as_ref().map(|answer| (offer.seat, answer)))
    }

    /// The seats that have not answered yet, in the order of the offers.
    pub fn awaiting(&self) -> impl Iterator<Item = SeatId> {
        self.offers
            .iter()
            .filter(|offer| offer.answer.is_none())
            .map(|offer| offer.seat)
    }
}
