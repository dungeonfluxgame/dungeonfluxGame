//! What the table asks each seat and what the seats say back, and the lines the table has heard.
//!
//! A runner sets a seat's ask (a prompt and suggested lines) and waits for its next line; the
//! seat's phone answers with a line of its own words, which goes to the seat's inbox as written and
//! ends the ask. The lines the table heard (every character's spoken line and the narrator's) are
//! kept, oldest first, for the phones' story. All of it lives in memory: a host restart forgets it,
//! and a runner sets its ask again. Every list is bounded (`limits`).
//!
//! An ask may go to several seats at once (a party ask): it is one ask, with one id, shown on every
//! seat asked. The first seat to answer it answers for the party: the ask ends on every seat, and
//! each other seat keeps who answered and what they said (`answered_of`) until its next ask.

use std::collections::{BTreeMap, VecDeque};

use df_playground::seat::wire::{
    AskKind, SayKind, SeatAction, SeatAsk, SeatOutcome, SeatThought, StoryLine, SuggestedLine,
};

use super::request::SeatError;
use crate::limits::{MAXIMUM_INBOX_LINES, MAXIMUM_SAID_IDS, MAXIMUM_STORY_LINES};
use df_playground::fixture::RollView;

/// A line a seat said, waiting for the runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InboxLine {
    /// The ask the line answers; none for talk.
    pub(crate) ask: Option<u64>,
    pub(crate) kind: SayKind,
    /// The thing of the ask's card the answer is, by id.
    pub(crate) thing: Option<String>,
    pub(crate) text: String,
    /// The character the line is said to, by persona id.
    pub(crate) to: Option<String>,
    /// When the line was taken, in milliseconds since the Unix epoch (the host's wall clock), so a
    /// runner measures how long it waited.
    pub(crate) at_ms: u64,
}

/// A spell a seat's hero cast (`SeatAction::Cast`), for the runner to narrate: the spell's id and
/// name, the level it was cast at (0 for a cantrip), whether as a Ritual, and what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CastNote {
    pub(crate) spell: String,
    pub(crate) name: String,
    pub(crate) level: u8,
    pub(crate) ritual: bool,
    pub(crate) summary: String,
}

/// What waits in a seat's inbox for the runner: a line it said, or a spell its hero cast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InboxEntry {
    Line(InboxLine),
    Cast { note: CastNote, at_ms: u64 },
}

impl InboxEntry {
    /// When the host took it, in milliseconds since the Unix epoch.
    fn at_ms(&self) -> u64 {
        match self {
            Self::Line(line) => line.at_ms,
            Self::Cast { at_ms, .. } => *at_ms,
        }
    }

    /// Whether it is talk (a line of the seat's own, not an answer).
    fn is_talk(&self) -> bool {
        matches!(self, Self::Line(line) if line.kind == SayKind::Talk)
    }
}

/// A line a seat says (`SeatAction::Answer` or `SeatAction::Talk`): an answer to the ask `ask`
/// (the one the phone showed) or talk of its own, said to `to` when it names someone, about the
/// thing `thing` of a card when it is a card's line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) text: String,
    pub(crate) to: Option<String>,
    pub(crate) kind: SayKind,
    pub(crate) thing: Option<String>,
    pub(crate) ask: Option<u64>,
}

/// What a runner asks a seat: the kind of answer, the prompt, the suggested lines and a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Asking {
    pub(crate) kind: AskKind,
    pub(crate) prompt: String,
    pub(crate) cards: Vec<SuggestedLine>,
    pub(crate) note: Option<String>,
}

/// A party ask answered from another seat: the ask, the seat that answered and its words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Answered {
    pub(crate) ask: u64,
    pub(crate) seat: u64,
    pub(crate) said: String,
}

/// A line the table heard: by a character (its id) or the narrator.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Heard {
    id: u64,
    speaker: Option<String>,
    text: String,
}

/// Whether a seat action may start (`SeatTalk::begin`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Begun {
    /// Its id was taken before: what it came to, or why the repeat is refused (`said_changed`,
    /// `in_flight`).
    Before(Result<SeatOutcome, SeatError>),
    /// It is being done now.
    Started,
}

/// The asks, the inboxes and the lines heard.
#[derive(Debug, Default)]
pub(crate) struct SeatTalk {
    asks: BTreeMap<u64, SeatAsk>,
    /// The seats each party ask (an ask of more than one seat) went to, by the ask's id, while
    /// one of them still holds it.
    parties: BTreeMap<u64, Vec<u64>>,
    /// Each seat's party ask answered from another seat, until the seat's next ask.
    answered: BTreeMap<u64, Answered>,
    inboxes: BTreeMap<u64, VecDeque<InboxEntry>>,
    /// Each seat's actions taken, by the phone's id for them, with what they came to, oldest
    /// first: a repeat is answered from here (`done_before`).
    said: BTreeMap<u64, VecDeque<(SeatAction, SeatOutcome)>>,
    /// Each seat's actions being done now, by the phone's id for them (`begin` to `finish`).
    doing: BTreeMap<u64, std::collections::BTreeSet<u64>>,
    heard: VecDeque<Heard>,
    /// Each character's latest thought that may be shown, by persona id (kept while the host
    /// runs; none after a restart until the next perception).
    /// The seats whose players turned talking by voice on.
    stt_on: std::collections::BTreeSet<u64>,
    /// A word for each seat from the table (a runner's `players note`), shown until the next ask.
    notices: BTreeMap<u64, String>,
    /// Each seat's last called roll, by its id, with the card the shared screen showed for it once
    /// it showed one.
    rolls: BTreeMap<u64, (String, Option<RollView>)>,
    /// With each, the picture it was had on, when one.
    thoughts: BTreeMap<String, (Option<String>, SeatThought)>,
    next_ask: u64,
    next_heard: u64,
}

impl SeatTalk {
    /// Talk whose first ask is numbered `first`: seeded from the wall clock at boot, so an ask id
    /// never repeats one an earlier host gave (a phone holding an old ask's id cannot answer a
    /// new one with it).
    pub(crate) fn seeded(first: u64) -> Self {
        Self {
            next_ask: first.saturating_sub(1),
            ..Self::default()
        }
    }

    /// The id the next ask gets.
    pub(crate) fn first_ask(&self) -> u64 {
        self.next_ask.saturating_add(1)
    }

    /// Keeps `thought`, had on `picture` (when one), as `persona`'s latest one a seat may show.
    pub(crate) fn think(&mut self, persona: &str, picture: Option<String>, thought: SeatThought) {
        self.thoughts.insert(persona.to_owned(), (picture, thought));
    }

    /// `persona`'s latest thought a seat may show while `shown` is the room on the shared screen
    /// (none: whatever it shows): a thought had on another room is of a scene that has passed
    /// (the show after a road).
    pub(crate) fn thought_of(&self, persona: &str, shown: Option<&str>) -> Option<SeatThought> {
        self.thoughts
            .get(persona)
            .filter(|(picture, _)| match (picture, shown) {
                (Some(picture), Some(shown)) => picture == shown,
                _ => true,
            })
            .map(|(_, thought)| thought.clone())
    }

    /// Asks `seats` something new, as one ask, replacing what each was asked; the ask's id. More
    /// than one seat makes it a party ask.
    pub(crate) fn ask(&mut self, seats: &[u64], mut asked: Asking) -> u64 {
        // One card for each thing: a thing listed twice (designed and noticed) is offered once.
        let mut things: Vec<String> = Vec::new();
        asked.cards.retain(|card| match &card.thing {
            Some(thing) if things.contains(thing) => false,
            Some(thing) => {
                things.push(thing.clone());
                true
            }
            None => true,
        });
        self.next_ask = self.next_ask.saturating_add(1);
        let id = self.next_ask;
        let ask = SeatAsk {
            id,
            kind: asked.kind,
            prompt: asked.prompt,
            cards: asked.cards,
            note: asked.note,
        };
        for seat in seats {
            // A notice is for the moment before this ask: the new ask replaces it.
            self.notices.remove(seat);
            self.answered.remove(seat);
            self.asks.insert(*seat, ask.clone());
        }
        // A party ask no seat holds any more is forgotten, so the list stays as small as the asks.
        let asks = &self.asks;
        self.parties
            .retain(|party, _| asks.values().any(|open| open.id == *party));
        if seats.len() > 1 {
            self.parties.insert(id, seats.to_vec());
        }
        id
    }

    /// `seat`'s party ask answered from another seat, until its next ask.
    pub(crate) fn answered_of(&self, seat: u64) -> Option<&Answered> {
        self.answered.get(&seat)
    }

    /// Forgets what `seat` was asked and the lines it said that no runner took: the seat changed
    /// hands (it left, or another key took it).
    pub(crate) fn forget_seat(&mut self, seat: u64) {
        self.asks.remove(&seat);
        self.answered.remove(&seat);
        self.inboxes.remove(&seat);
        self.said.remove(&seat);
        self.notices.remove(&seat);
        self.rolls.remove(&seat);
        self.stt_on.remove(&seat);
    }

    /// Notes that the roll `roll_id` was called for `seat`'s hero: the shared screen's card of that
    /// roll is the seat's last roll.
    pub(crate) fn roll_called(&mut self, seat: u64, roll_id: &str) {
        if self.rolls.get(&seat).is_none_or(|(id, _)| id != roll_id) {
            self.rolls.insert(seat, (roll_id.to_owned(), None));
        }
    }

    /// `seat`'s last roll as the shared screen showed it: the card `shown` when it is the seat's
    /// called roll (kept from then on), else the one kept.
    pub(crate) fn last_roll(&mut self, seat: u64, shown: Option<&RollView>) -> Option<RollView> {
        let (id, kept) = self.rolls.get_mut(&seat)?;
        if let Some(shown) = shown.filter(|shown| shown.id == *id) {
            *kept = Some(shown.clone());
        }
        kept.clone()
    }

    /// Gives `seat` a word from the table, in place of the one it had, until the next ask.
    pub(crate) fn note(&mut self, seat: u64, text: &str) {
        self.notices.insert(seat, text.to_owned());
    }

    /// The word from the table `seat` shows now, if any.
    pub(crate) fn notice_of(&self, seat: u64) -> Option<String> {
        self.notices.get(&seat).cloned()
    }

    /// Turns talking by voice on or off for `seat`.
    pub(crate) fn set_stt(&mut self, seat: u64, on: bool) {
        if on {
            self.stt_on.insert(seat);
        } else {
            self.stt_on.remove(&seat);
        }
    }

    /// Whether `seat` talks by voice now.
    pub(crate) fn stt_on(&self, seat: u64) -> bool {
        self.stt_on.contains(&seat)
    }

    /// Stops asking `seat`; whether it was asked something.
    pub(crate) fn clear_ask(&mut self, seat: u64) -> bool {
        self.asks.remove(&seat).is_some()
    }

    pub(crate) fn ask_of(&self, seat: u64) -> Option<&SeatAsk> {
        self.asks.get(&seat)
    }

    /// Starts `seat`'s `action`, in one step with the look for a repeat, so two copies of one
    /// action never both run: `Before` with what it came to when it was done (or `said_changed`
    /// when its id came with another body), `in_flight` while a copy is being done, else
    /// `Started`, after which `finish` must follow.
    pub(crate) fn begin(&mut self, seat: u64, action: &SeatAction) -> Begun {
        if let Some(before) = self.done_before(seat, action) {
            return Begun::Before(before);
        }
        let said = action.said();
        if !self.doing.entry(seat).or_default().insert(said) {
            return Begun::Before(Err(SeatError::InFlight { said }));
        }
        Begun::Started
    }

    /// `seat`'s action with the phone's id `said` is no longer being done (done, refused or
    /// stopped).
    pub(crate) fn finish(&mut self, seat: u64, said: u64) {
        if let Some(doing) = self.doing.get_mut(&seat) {
            doing.remove(&said);
            if doing.is_empty() {
                self.doing.remove(&seat);
            }
        }
    }

    /// What `seat`'s action with `action`'s id came to, when one was taken: the same outcome for a
    /// repeat, or a refusal when the same id came with another body.
    fn done_before(
        &self,
        seat: u64,
        action: &SeatAction,
    ) -> Option<Result<SeatOutcome, SeatError>> {
        let (before, outcome) = self
            .said
            .get(&seat)?
            .iter()
            .find(|(before, _)| before.said() == action.said())?;
        Some(if before == action {
            Ok(outcome.clone())
        } else {
            Err(SeatError::SaidChanged {
                said: action.said(),
            })
        })
    }

    /// Keeps what `seat`'s `action` came to, so a repeat of it is answered as before (the oldest
    /// forgotten first when the seat's list is full).
    pub(crate) fn remember(&mut self, seat: u64, action: SeatAction, outcome: SeatOutcome) {
        let taken = self.said.entry(seat).or_default();
        if taken.len() >= MAXIMUM_SAID_IDS {
            taken.pop_front();
        }
        taken.push_back((action, outcome));
    }

    /// Takes `line` as `seat`'s, which waits in the seat's inbox (the oldest forgotten first when
    /// it is full). An answer ends what the seat is asked and gives the answered ask's id; talk
    /// leaves the ask open and gives none. A thing must be one a card of the open ask is about,
    /// and only an answer names one.
    pub(crate) fn say(
        &mut self,
        seat: u64,
        line: Line,
        at_ms: u64,
    ) -> Result<Option<u64>, SeatError> {
        let Line {
            text,
            to,
            kind,
            thing,
            ask: answering,
        } = line;
        // An answer to an ask that was replaced (a stale card) is not the new ask's answer.
        if kind == SayKind::Answer
            && let Some(answering) = answering
            && self.asks.get(&seat).is_none_or(|open| open.id != answering)
        {
            return Err(SeatError::StaleAsk { ask: answering });
        }
        let text = text.trim();
        if text.is_empty() {
            return Err(SeatError::EmptyLine);
        }
        if let Some(thing) = &thing {
            let offered = kind == SayKind::Answer
                && self.asks.get(&seat).is_some_and(|ask| {
                    ask.cards
                        .iter()
                        .any(|card| card.thing.as_ref() == Some(thing))
                });
            if !offered {
                return Err(SeatError::UnknownThing {
                    thing: thing.clone(),
                });
            }
        }
        let ask = match kind {
            SayKind::Answer => Some(self.asks.remove(&seat).ok_or(SeatError::NothingAsked)?.id),
            SayKind::Talk => None,
        };
        if let Some(id) = ask {
            self.answer_for_party(seat, id, text);
        }
        self.put_in_inbox(
            seat,
            InboxEntry::Line(InboxLine {
                ask,
                kind,
                thing,
                text: text.to_owned(),
                to,
                at_ms,
            }),
        );
        Ok(ask)
    }

    /// Puts the spell `note` `seat`'s hero cast in the seat's inbox, for the runner.
    pub(crate) fn cast(&mut self, seat: u64, note: CastNote, at_ms: u64) {
        self.put_in_inbox(seat, InboxEntry::Cast { note, at_ms });
    }

    /// Puts `entry` last in `seat`'s inbox, the oldest forgotten first when it is full.
    fn put_in_inbox(&mut self, seat: u64, entry: InboxEntry) {
        let inbox = self.inboxes.entry(seat).or_default();
        if inbox.len() >= MAXIMUM_INBOX_LINES {
            inbox.pop_front();
        }
        inbox.push_back(entry);
    }

    /// The party ask `id` answered from `seat` with `said`: it ends on every other seat it went to,
    /// and each is told who answered.
    fn answer_for_party(&mut self, seat: u64, id: u64, said: &str) {
        let Some(party) = self.parties.remove(&id) else {
            return;
        };
        for other in party.into_iter().filter(|other| *other != seat) {
            if self.asks.get(&other).is_some_and(|open| open.id == id) {
                self.asks.remove(&other);
                self.answered.insert(
                    other,
                    Answered {
                        ask: id,
                        seat,
                        said: said.to_owned(),
                    },
                );
            }
        }
    }

    /// How many Talk lines of `seat` wait in its inbox, not yet taken.
    pub(crate) fn talk_waiting(&self, seat: u64) -> u32 {
        let waiting = self.inboxes.get(&seat).map_or(0, |inbox| {
            inbox.iter().filter(|entry| entry.is_talk()).count()
        });
        u32::try_from(waiting).unwrap_or(u32::MAX)
    }

    /// Drops the Talk lines of `seat` no one took (its day ended); how many.
    pub(crate) fn drop_talk(&mut self, seat: u64) -> u32 {
        let dropped = self.talk_waiting(seat);
        if let Some(inbox) = self.inboxes.get_mut(&seat) {
            inbox.retain(|entry| !entry.is_talk());
        }
        dropped
    }

    /// The oldest entry waiting in the inboxes of `seats` (the earlier seat of the list first when
    /// two were taken at once), taken out of it, with its seat.
    pub(crate) fn take_line(&mut self, seats: &[u64]) -> Option<(u64, InboxEntry)> {
        let seat = seats
            .iter()
            .filter_map(|seat| {
                let entry = self.inboxes.get(seat)?.front()?;
                Some((entry.at_ms(), *seat))
            })
            .min_by_key(|(at_ms, _)| *at_ms)
            .map(|(_, seat)| seat)?;
        let line = self.inboxes.get_mut(&seat)?.pop_front()?;
        Some((seat, line))
    }

    /// Notes a line the table heard; the oldest are forgotten first.
    pub(crate) fn hear(&mut self, speaker: Option<&str>, text: &str) {
        if self.heard.len() >= MAXIMUM_STORY_LINES {
            self.heard.pop_front();
        }
        self.next_heard = self.next_heard.saturating_add(1);
        self.heard.push_back(Heard {
            id: self.next_heard,
            speaker: speaker.map(str::to_owned),
            text: text.to_owned(),
        });
    }

    /// The lines heard, oldest first, as the seat whose character is `persona` reads them.
    pub(crate) fn story_lines(&self, persona: &str) -> Vec<StoryLine> {
        self.heard
            .iter()
            .map(|heard| StoryLine {
                id: heard.id,
                you: heard.speaker.as_deref() == Some(persona),
                speaker: heard.speaker.clone(),
                text: heard.text.clone(),
            })
            .collect()
    }
}
