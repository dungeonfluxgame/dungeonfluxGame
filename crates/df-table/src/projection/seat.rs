//! One seat's view: its hero, the status, the story it may read, its choices and the note of what
//! only it knows.
//!
//! It owns `SeatView`, whose JSON is the playground's `PhoneScene` (`hero`, `status`, `narration`,
//! `suggested` and, when there are some, `groups`, `unavailable` and `note`), `OfferView` (a choice
//! with the id a tap sends back, which the fixture shape cannot carry, so offers travel beside the
//! view) and the build of both. The story a seat reads is the public lines and its own private
//! ones, merged in the order they happened and cut to the last `PHONE_NARRATION_LINES` lines, a
//! decline's reason counting as one, and to `NARRATION_BUDGET_BYTES`: the newest entry always
//! shows, an entry is never cut in half (its structured readout stands first, C.7), and the count
//! of older entries left off is typed data. The data is filed by audience when it is published, so
//! another seat's line, fact, offer or decline is never even looked at. The note is the facts only
//! this seat knows, newest first within `NOTE_BUDGET_BYTES`, one per line, with the count left off
//! typed too; reading them is the one place a `Secret` is exposed, and only for the seat it belongs
//! to. A decline shows only while it answers the table's current revision, and its alternatives
//! become tappable offers with ids `alternative-N`. The narration is never empty: with no story it
//! shows the seat's machine readout, and failing that a plain sentence from the table's facts
//! (H.15). It never builds a view for a seat that is not at the table: that is a typed error, not
//! an empty view.

use std::collections::BTreeMap;

use serde::{Serialize, Serializer};

use crate::identity::{EvidenceRef, SeatId};
use crate::private_data::{LengthClass, Measured};
use crate::projection::attachments::{OfferEntry, OfferId, StoryEntry};
use crate::projection::bounds::{NARRATION_BUDGET_BYTES, NOTE_BUDGET_BYTES, PHONE_NARRATION_LINES};
use crate::projection::record::ProjectionWork;
use crate::projection::status::ProjectionStatus;
use crate::projection::text::NonEmptyText;
use crate::projection::{Attachments, ProjectionError, overview, status_of};
use crate::table_state::TableSnapshot;

/// The lines of a phone's story: at least one, so a phone never has an empty page (H.15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NarrationLines {
    first: NonEmptyText,
    rest: Vec<NonEmptyText>,
}

impl NarrationLines {
    /// The lines in reading order: oldest first, newest last.
    pub fn iter(&self) -> impl Iterator<Item = &NonEmptyText> {
        std::iter::once(&self.first).chain(&self.rest)
    }

    pub fn len(&self) -> usize {
        1 + self.rest.len()
    }

    /// Always false: there is at least one line.
    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn newest(&self) -> &NonEmptyText {
        self.rest.last().unwrap_or(&self.first)
    }

    fn from_lines(mut lines: Vec<NonEmptyText>, fallback: impl FnOnce() -> NonEmptyText) -> Self {
        if lines.is_empty() {
            return Self {
                first: fallback(),
                rest: Vec::new(),
            };
        }
        let first = lines.remove(0);
        Self { first, rest: lines }
    }
}

impl Serialize for NarrationLines {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}

/// A choice as the phone draws it, without the id a tap sends back (see `OfferView`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChoiceView {
    pub label: NonEmptyText,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explain: Option<NonEmptyText>,
}

/// A named group of choices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GroupView {
    pub name: NonEmptyText,
    pub choices: Vec<ChoiceView>,
}

/// A choice the hero cannot take now, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnavailableView {
    pub label: NonEmptyText,
    pub reason: NonEmptyText,
}

/// One thing the seat may do now, with the id a tap sends back (`MachineOffer`). It sits beside
/// the view, not in it, because the playground's choice has no id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfferView {
    pub offer_id: OfferId,
    pub label: NonEmptyText,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explain: Option<NonEmptyText>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<NonEmptyText>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<NonEmptyText>,
}

impl From<&OfferEntry> for OfferView {
    fn from(entry: &OfferEntry) -> Self {
        Self {
            offer_id: entry.offer_id.clone(),
            label: entry.label.clone(),
            explain: entry.explain.clone(),
            group: entry.group.clone(),
            unavailable_reason: entry.unavailable_reason.clone(),
        }
    }
}

/// Where a hidden hero stands, shown on its own phone only (R06): the party cannot see it, but its
/// player must know where it is. Canvas units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PositionOverlay {
    pub x: f32,
    pub y: f32,
}

/// What only this seat knows, shown as a private note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteView {
    pub text: NonEmptyText,
}

/// What one seat's phone shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SeatView {
    pub hero: NonEmptyText,
    pub status: ProjectionStatus,
    pub narration: NarrationLines,
    pub suggested: Vec<ChoiceView>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<GroupView>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unavailable: Vec<UnavailableView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<NoteView>,
}

/// A seat view and what was learned building it.
pub(super) struct SeatBuild {
    pub(super) view: SeatView,
    pub(super) offers: Vec<OfferView>,
    pub(super) overlay: Option<PositionOverlay>,
    pub(super) readout: NonEmptyText,
    pub(super) facts_shown: usize,
    pub(super) older_story_entries: usize,
    pub(super) older_facts: usize,
    pub(super) newest_evidence: Option<EvidenceRef>,
    pub(super) newest_line_length: LengthClass,
}

/// The seat's hero name, or "Seat N" while it has none (a seat in creation).
fn hero_of(snapshot: &TableSnapshot, seat: SeatId) -> NonEmptyText {
    match snapshot.hero_of(seat) {
        Some(hero) => NonEmptyText::or_fallback(hero.name().as_str()),
        None => NonEmptyText::or_fallback(&format!("Seat {seat}")),
    }
}

/// The lines of one story entry in reading order: the structured readout first, then the words.
fn entry_lines(entry: &StoryEntry) -> Vec<NonEmptyText> {
    let line = &entry.line;
    let mut lines = vec![line.readout.clone()];
    if let Some(narration) = &line.narration
        && let Ok(words) = NonEmptyText::new(narration.expose())
    {
        let said = match &line.speaker {
            Some(speaker) => format!("{}: {}", speaker.as_str(), words.as_str()),
            None => words.as_str().to_owned(),
        };
        lines.push(NonEmptyText::or_fallback(&said));
    }
    lines
}

/// What the story window held.
struct StoryWindow {
    lines: Vec<NonEmptyText>,
    newest_evidence: Option<EvidenceRef>,
    newest_readout: Option<NonEmptyText>,
    entries_shown: usize,
}

/// The newest story entries the seat may read that fit `line_limit` lines and
/// `NARRATION_BUDGET_BYTES`, oldest first. The public queue and the seat's own are merged by
/// sequence from the newest end, so the work is bounded by the lines shown and not by the length of
/// the story. An entry is shown whole or not at all, except that the newest always shows.
fn story_window(
    attachments: &Attachments,
    seat: SeatId,
    line_limit: usize,
    work: &mut ProjectionWork,
) -> StoryWindow {
    let empty = std::collections::VecDeque::new();
    let mut public = attachments.public_story().iter().rev().peekable();
    let mut private = attachments
        .seat_story(seat)
        .unwrap_or(&empty)
        .iter()
        .rev()
        .peekable();
    let mut chunks: Vec<Vec<NonEmptyText>> = Vec::new();
    let mut window = StoryWindow {
        lines: Vec::new(),
        newest_evidence: None,
        newest_readout: None,
        entries_shown: 0,
    };
    let mut lines_taken = 0;
    let mut bytes_taken = 0;
    loop {
        let from_public = match (public.peek(), private.peek()) {
            (Some(shared), Some(own)) => shared.sequence > own.sequence,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        let next = if from_public {
            public.peek()
        } else {
            private.peek()
        };
        let Some(entry) = next.copied() else {
            break;
        };
        work.story_entries_visited += 1;
        let lines = entry_lines(entry);
        let bytes: usize = lines.iter().map(|line| line.as_str().len()).sum();
        let fits = lines_taken + lines.len() <= line_limit
            && bytes_taken + bytes <= NARRATION_BUDGET_BYTES;
        if !chunks.is_empty() && !fits {
            break;
        }
        if chunks.is_empty() {
            window.newest_evidence = entry.line.evidence;
            window.newest_readout = Some(entry.line.readout.clone());
        }
        lines_taken += lines.len();
        bytes_taken += bytes;
        window.entries_shown += 1;
        chunks.push(lines);
        if from_public {
            public.next();
        } else {
            private.next();
        }
    }
    window.lines = chunks.into_iter().rev().flatten().collect();
    window
}

/// The seat's private facts as one note, newest first within the budget and shown oldest first,
/// one fact per line; how many are shown and how many older ones are left off.
fn note_of(
    attachments: &Attachments,
    seat: SeatId,
    work: &mut ProjectionWork,
) -> (Option<NoteView>, usize, usize) {
    let Some(facts) = attachments.seat_facts(seat) else {
        return (None, 0, 0);
    };
    let mut texts: Vec<&str> = Vec::new();
    let mut bytes = 0;
    let mut blanks = 0;
    for fact in facts.newest_first() {
        work.facts_visited += 1;
        let text = fact.text.expose();
        if text.trim().is_empty() {
            blanks += 1;
            continue;
        }
        if !texts.is_empty() && bytes + text.len() > NOTE_BUDGET_BYTES {
            break;
        }
        bytes += text.len();
        texts.push(text);
    }
    let shown = texts.len();
    let older = facts.len().saturating_sub(shown + blanks);
    texts.reverse();
    let note = NonEmptyText::new(&texts.join("\n"))
        .ok()
        .map(|text| NoteView { text });
    (note, shown, older)
}

/// The overlay of a hero the party cannot see and that stands somewhere: its own seat's alone.
fn overlay_of(snapshot: &TableSnapshot, seat: SeatId) -> Option<PositionOverlay> {
    let hero = snapshot.hero_of(seat)?;
    hero.visibility().chip()?;
    let placement = hero.placement()?;
    Some(PositionOverlay {
        x: placement.x_units(),
        y: placement.y_units(),
    })
}

/// The offers and the three lists the phone draws from them. A decline's alternatives follow as
/// offers with ids `alternative-N`.
fn offers_of(attachments: &Attachments, seat: SeatId, alternatives: &[String]) -> Vec<OfferView> {
    let mut offers: Vec<OfferView> = attachments
        .seat_offers(seat)
        .iter()
        .map(OfferView::from)
        .collect();
    for (index, label) in alternatives.iter().enumerate() {
        let Ok(offer_id) = OfferId::new(&format!("alternative-{}", index + 1)) else {
            continue;
        };
        if offers.iter().any(|offer| offer.offer_id == offer_id) {
            continue;
        }
        offers.push(OfferView {
            offer_id,
            label: NonEmptyText::or_fallback(label),
            explain: None,
            group: None,
            unavailable_reason: None,
        });
    }
    offers
}

fn choice_of(offer: &OfferView) -> ChoiceView {
    ChoiceView {
        label: offer.label.clone(),
        explain: offer.explain.clone(),
    }
}

/// Splits the offers into what the fixture shape draws: the suggested ones, the named groups and
/// the unavailable ones with their reasons.
fn lists_of(offers: &[OfferView]) -> (Vec<ChoiceView>, Vec<GroupView>, Vec<UnavailableView>) {
    let mut suggested = Vec::new();
    let mut groups: BTreeMap<&str, Vec<ChoiceView>> = BTreeMap::new();
    let mut unavailable = Vec::new();
    for offer in offers {
        match (&offer.unavailable_reason, &offer.group) {
            (Some(reason), _) => unavailable.push(UnavailableView {
                label: offer.label.clone(),
                reason: reason.clone(),
            }),
            (None, Some(group)) => groups
                .entry(group.as_str())
                .or_default()
                .push(choice_of(offer)),
            (None, None) => suggested.push(choice_of(offer)),
        }
    }
    let groups = groups
        .into_iter()
        .map(|(name, choices)| GroupView {
            name: NonEmptyText::or_fallback(name),
            choices,
        })
        .collect();
    (suggested, groups, unavailable)
}

/// Builds one seat's view, or says the seat is not at the table.
pub(super) fn build(
    snapshot: &TableSnapshot,
    attachments: &Attachments,
    seat: SeatId,
    work: &mut ProjectionWork,
) -> Result<SeatBuild, ProjectionError> {
    if snapshot.seat(seat).is_none() {
        return Err(ProjectionError::SeatNotAtTable { seat });
    }
    let notice = attachments.notice(seat, snapshot.revision());
    let line_limit = PHONE_NARRATION_LINES - usize::from(notice.is_some());
    let window = story_window(attachments, seat, line_limit, work);
    let total_entries = attachments.public_story().len()
        + attachments.seat_story(seat).map_or(0, |queue| queue.len());
    let mut lines = window.lines;
    if let Some(decline) = notice {
        lines.push(NonEmptyText::or_fallback(decline.reason()));
    }
    let alternatives = notice.map_or(&[][..], |decline| decline.alternatives());
    let offers = offers_of(attachments, seat, alternatives);
    let (suggested, groups, unavailable) = lists_of(&offers);
    let narration = NarrationLines::from_lines(lines, || {
        attachments
            .seat_readout(seat)
            .cloned()
            .unwrap_or_else(|| overview(snapshot, attachments))
    });
    let (note, facts_shown, older_facts) = note_of(attachments, seat, work);
    let readout = attachments
        .seat_readout(seat)
        .cloned()
        .or(window.newest_readout)
        .unwrap_or_else(|| overview(snapshot, attachments));
    let newest_line_length = LengthClass::of_length(narration.newest().length_in_chars());
    Ok(SeatBuild {
        view: SeatView {
            hero: hero_of(snapshot, seat),
            status: status_of(snapshot, attachments),
            narration,
            suggested,
            groups,
            unavailable,
            note,
        },
        offers,
        overlay: overlay_of(snapshot, seat),
        readout,
        facts_shown,
        older_story_entries: total_entries.saturating_sub(window.entries_shown),
        older_facts,
        newest_evidence: window.newest_evidence,
        newest_line_length,
    })
}
