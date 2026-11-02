//! What the owners publish for projection beside the table's facts (RES30).
//!
//! It owns `Attachments`, the bundle a view call reads with a `TableSnapshot`: the story (the
//! table's public lines and each seat's private ones, newest last), the facts a seat alone knows,
//! the place names of scenes, the machines' state labels, the machines' readouts, a seat's offers
//! and latest decline, and the pipeline's activity. Owners publish into it at commit or job-result
//! time and a view call never writes to it, so projection reads no file, provider or clock.
//!
//! It is shared the way the table's facts are: every collection sits behind an `Arc` and a publish
//! copies only the collection it changes, and only while a snapshot still holds the old one. So
//! `snapshot()` is a handful of pointer copies, a snapshot never sees a later publish, and the
//! owner is never blocked by a view in progress (RES30, H.14). Every publish raises the
//! `attachment_version` of RES30 (`version_for`) for the audiences that can see it, so a watcher
//! can tell an attachment-only change from no change at the same table revision without learning
//! that another seat was told something.
//!
//! Everything is filed by audience when it goes in: the public story, and a queue, a fact set, an
//! offer list and a notice per seat. A seat's view therefore reads only the public lines and its
//! own, and the filtering is the shape of the data and not a check that could be forgotten.
//! Knowledge that everyone may know reaches views through the story (a public line), so public
//! facts have no list of their own. Text a person typed or the storyteller wrote stays in
//! `Narration` and `Secret<String>`, which never print. Nothing is write-only: a notice, a readout,
//! an offer list, a fact, a place and a left seat's private data can each be cleared (`forget_seat`
//! is called in the commit that applies `SeatLeft`, RES36), and a decline notice belongs to the
//! table revision it answers, so it expires with the next commit. Every collection is bounded by a
//! named large limit (`AttachmentLimits`, whose default is the constants in `bounds`), every
//! published text by `MAXIMUM_TEXT_BYTES` (the wire frame, #1686), and an owner that goes over a
//! bound gets a typed error. It never decides what an audience may see beyond filing: `projection`
//! builds the views.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::Arc;

use serde::Serialize;

use crate::element::name_type;
use crate::identity::{EvidenceRef, FactId, MachineId, SceneId, SeatId, SessionRevision};
use crate::outcome::Decline;
use crate::ports::Audience;
use crate::private_data::{Narration, Secret};
use crate::projection::bounds::{
    MAXIMUM_FACTS_PER_SEAT, MAXIMUM_OFFERS_PER_SEAT, MAXIMUM_PLACES, MAXIMUM_SEATS_WITH_ENTRIES,
    MAXIMUM_STATE_LABELS, MAXIMUM_STORY_ENTRIES, MAXIMUM_TEXT_BYTES, OFFERS_BUDGET_BYTES,
};
use crate::projection::status::ProjectionStatus;
use crate::projection::text::NonEmptyText;
use crate::table_state::DisplayName;

name_type!(
    /// The id a tap on an offer sends back (`MachineOffer.offer_id`).
    OfferId
);

/// The `attachment_version` of RES30 as one audience sees it: it rises with every publish or clear
/// that audience can see.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct AttachmentVersion(u64);

impl AttachmentVersion {
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for AttachmentVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Why something could not be published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentError {
    TooManySeats,
    TooManyScenes,
    TooManyFacts,
    TooManyOffers,
    /// Two offers of one seat have the same id, so a tap could not name one of them.
    DuplicateOffer,
    TooManyMachines,
    /// A text is longer than `MAXIMUM_TEXT_BYTES`, or a seat's offers together are longer than
    /// `OFFERS_BUDGET_BYTES`: either would not fit the wire frame.
    TextTooLong,
    /// The version counter ran out.
    VersionExhausted,
}

impl fmt::Display for AttachmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManySeats => formatter.write_str("too many seats have entries"),
            Self::TooManyScenes => formatter.write_str("too many scenes have a name"),
            Self::TooManyFacts => formatter.write_str("a seat knows too many facts"),
            Self::TooManyOffers => formatter.write_str("a seat has too many offers"),
            Self::DuplicateOffer => formatter.write_str("two offers have the same id"),
            Self::TooManyMachines => formatter.write_str("too many machines have a state label"),
            Self::TextTooLong => formatter.write_str("a text is too long for the wire frame"),
            Self::VersionExhausted => formatter.write_str("the attachment version is exhausted"),
        }
    }
}

impl std::error::Error for AttachmentError {}

fn within(text: &str) -> Result<(), AttachmentError> {
    if text.len() > MAXIMUM_TEXT_BYTES {
        return Err(AttachmentError::TextTooLong);
    }
    Ok(())
}

/// One line of the story as its owner hands it over: the structured readout that always stands
/// first, the storyteller's words if they arrived, who said it and the evidence reference of the
/// specimen in the diagnostic store (RES36).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryLine {
    pub(super) speaker: Option<DisplayName>,
    pub(super) readout: NonEmptyText,
    pub(super) narration: Option<Narration>,
    pub(super) evidence: Option<EvidenceRef>,
}

impl StoryLine {
    pub fn new(readout: NonEmptyText) -> Self {
        Self {
            speaker: None,
            readout,
            narration: None,
            evidence: None,
        }
    }

    pub fn spoken_by(mut self, speaker: DisplayName) -> Self {
        self.speaker = Some(speaker);
        self
    }

    pub fn with_narration(mut self, narration: Narration) -> Self {
        self.narration = Some(narration);
        self
    }

    pub fn with_evidence(mut self, evidence: EvidenceRef) -> Self {
        self.evidence = Some(evidence);
        self
    }

    fn check(&self) -> Result<(), AttachmentError> {
        within(self.readout.as_str())?;
        match &self.narration {
            Some(narration) => within(narration.expose()),
            None => Ok(()),
        }
    }
}

/// A story line with its place in the story, so two queues merge into one order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StoryEntry {
    pub(super) sequence: u64,
    pub(super) line: StoryLine,
}

/// One thing a seat may do now (RES33 `ActionOffer`): the id a tap sends back, a label and, as the
/// owner of the legal choices decides, an explanation, a named group, or the reason it cannot be
/// taken now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferEntry {
    pub(super) offer_id: OfferId,
    pub(super) label: NonEmptyText,
    pub(super) explain: Option<NonEmptyText>,
    pub(super) group: Option<NonEmptyText>,
    pub(super) unavailable_reason: Option<NonEmptyText>,
}

impl OfferEntry {
    pub fn new(offer_id: OfferId, label: NonEmptyText) -> Self {
        Self {
            offer_id,
            label,
            explain: None,
            group: None,
            unavailable_reason: None,
        }
    }

    pub fn explained(mut self, explain: NonEmptyText) -> Self {
        self.explain = Some(explain);
        self
    }

    pub fn in_group(mut self, group: NonEmptyText) -> Self {
        self.group = Some(group);
        self
    }

    pub fn unavailable(mut self, reason: NonEmptyText) -> Self {
        self.unavailable_reason = Some(reason);
        self
    }

    pub(super) fn text_bytes(&self) -> usize {
        let optional = |text: &Option<NonEmptyText>| text.as_ref().map_or(0, |t| t.as_str().len());
        self.offer_id.as_str().len()
            + self.label.as_str().len()
            + optional(&self.explain)
            + optional(&self.group)
            + optional(&self.unavailable_reason)
    }
}

/// A decline a seat was shown, with the table revision it answers: it is shown while the table is
/// still at that revision and is gone after the next commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Notice {
    pub(super) decline: Decline,
    pub(super) at: SessionRevision,
}

/// A fact with the place in publishing order that makes the newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FactEntry {
    pub(super) sequence: u64,
    pub(super) text: Secret<String>,
}

/// The facts one seat knows, and the order they were learned in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SeatFacts {
    by_id: BTreeMap<FactId, FactEntry>,
    newest_first: BTreeMap<u64, FactId>,
}

impl SeatFacts {
    pub(super) fn len(&self) -> usize {
        self.by_id.len()
    }

    /// The facts from the newest to the oldest.
    pub(super) fn newest_first(&self) -> impl Iterator<Item = &FactEntry> {
        self.newest_first
            .values()
            .rev()
            .filter_map(|fact| self.by_id.get(fact))
    }

    fn set(&mut self, fact: FactId, sequence: u64, text: Secret<String>) {
        if let Some(old) = self.by_id.insert(fact, FactEntry { sequence, text }) {
            self.newest_first.remove(&old.sequence);
        }
        self.newest_first.insert(sequence, fact);
    }

    fn remove(&mut self, fact: &FactId) {
        if let Some(old) = self.by_id.remove(fact) {
            self.newest_first.remove(&old.sequence);
        }
    }
}

/// How much a bundle of attachments holds. The defaults are the large named constants of
/// `bounds`; a test or a host with a measurement may set smaller ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentLimits {
    /// Most story entries one queue keeps: the public one, and each seat's.
    pub story_entries: usize,
    pub facts_per_seat: usize,
    pub offers_per_seat: usize,
    /// Most seats that may have a story, facts, offers, a readout or a notice.
    pub seats: usize,
    pub places: usize,
    pub state_labels: usize,
}

impl Default for AttachmentLimits {
    fn default() -> Self {
        Self {
            story_entries: MAXIMUM_STORY_ENTRIES,
            facts_per_seat: MAXIMUM_FACTS_PER_SEAT,
            offers_per_seat: MAXIMUM_OFFERS_PER_SEAT,
            seats: MAXIMUM_SEATS_WITH_ENTRIES,
            places: MAXIMUM_PLACES,
            state_labels: MAXIMUM_STATE_LABELS,
        }
    }
}

type Story = Arc<VecDeque<Arc<StoryEntry>>>;

/// The owners' published state beside the table's facts. Cloning it is a snapshot (`snapshot`).
#[derive(Debug, Clone, PartialEq)]
pub struct Attachments {
    limits: AttachmentLimits,
    /// Orders the story and the facts across queues; never shown.
    sequence: u64,
    /// Counts the publishes every audience can see.
    public_version: u64,
    /// Counts the publishes only one seat can see, per seat.
    seat_versions: Arc<BTreeMap<SeatId, u64>>,
    places: Arc<BTreeMap<SceneId, NonEmptyText>>,
    state_labels: Arc<BTreeMap<MachineId, NonEmptyText>>,
    public_story: Story,
    seat_story: Arc<BTreeMap<SeatId, Story>>,
    seat_facts: Arc<BTreeMap<SeatId, Arc<SeatFacts>>>,
    seat_offers: Arc<BTreeMap<SeatId, Arc<Vec<OfferEntry>>>>,
    shared_readout: Option<NonEmptyText>,
    seat_readouts: Arc<BTreeMap<SeatId, NonEmptyText>>,
    notices: Arc<BTreeMap<SeatId, Notice>>,
    activity: ProjectionStatus,
}

impl Default for Attachments {
    fn default() -> Self {
        Self::with_limits(AttachmentLimits::default())
    }
}

/// Appends `entry`, dropping the oldest while the queue holds more than `maximum`.
fn push_bounded(queue: &mut Story, entry: StoryEntry, maximum: usize) {
    let queue = Arc::make_mut(queue);
    queue.push_back(Arc::new(entry));
    while queue.len() > maximum {
        queue.pop_front();
    }
}

fn check_room<V>(
    map: &BTreeMap<SeatId, V>,
    seat: SeatId,
    maximum: usize,
) -> Result<(), AttachmentError> {
    if map.len() >= maximum && !map.contains_key(&seat) {
        return Err(AttachmentError::TooManySeats);
    }
    Ok(())
}

impl Attachments {
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty bundle that holds at most `limits`.
    pub fn with_limits(limits: AttachmentLimits) -> Self {
        Self {
            limits,
            sequence: 0,
            public_version: 0,
            seat_versions: Arc::default(),
            places: Arc::default(),
            state_labels: Arc::default(),
            public_story: Arc::default(),
            seat_story: Arc::default(),
            seat_facts: Arc::default(),
            seat_offers: Arc::default(),
            shared_readout: None,
            seat_readouts: Arc::default(),
            notices: Arc::default(),
            activity: ProjectionStatus::Ok,
        }
    }

    /// The `attachment_version` of RES30 as `audience` sees it: it rises with every publish or
    /// clear that audience can see and with none that it cannot, so two states of the attachments
    /// at one table revision are told apart, and a watcher learns nothing of another seat's private
    /// publishes from it. The shared screen counts the public ones; a seat counts those and its
    /// own.
    pub fn version_for(&self, audience: Audience) -> AttachmentVersion {
        match audience {
            Audience::Public => AttachmentVersion(self.public_version),
            Audience::Seat(seat) => {
                let own = self.seat_versions.get(&seat).copied().unwrap_or(0);
                AttachmentVersion(self.public_version.saturating_add(own))
            }
        }
    }

    /// The attachments as they are now, for a view call to read while the owner keeps publishing.
    /// It copies a handful of pointers and no collection; a later publish copies only the
    /// collection it changes and leaves this snapshot as it was.
    pub fn snapshot(&self) -> Self {
        self.clone()
    }

    /// Counts a publish every audience can see. A publish does this first, so a refusal changes
    /// nothing.
    fn bump_public(&mut self) -> Result<(), AttachmentError> {
        self.public_version = self
            .public_version
            .checked_add(1)
            .ok_or(AttachmentError::VersionExhausted)?;
        Ok(())
    }

    /// Counts a publish only `seat` can see.
    fn bump_seat(&mut self, seat: SeatId) -> Result<(), AttachmentError> {
        let count = Arc::make_mut(&mut self.seat_versions)
            .entry(seat)
            .or_insert(0);
        *count = count
            .checked_add(1)
            .ok_or(AttachmentError::VersionExhausted)?;
        Ok(())
    }

    /// The next place in the story's order.
    fn next_sequence(&mut self) -> Result<u64, AttachmentError> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(AttachmentError::VersionExhausted)?;
        Ok(self.sequence)
    }

    /// Names a scene's place for the shared screen.
    pub fn set_place(
        &mut self,
        scene: SceneId,
        place: NonEmptyText,
    ) -> Result<(), AttachmentError> {
        within(place.as_str())?;
        if self.places.len() >= self.limits.places && !self.places.contains_key(&scene) {
            return Err(AttachmentError::TooManyScenes);
        }
        self.bump_public()?;
        Arc::make_mut(&mut self.places).insert(scene, place);
        Ok(())
    }

    /// Withdraws a scene's place name, which a closed scene no longer needs.
    pub fn forget_place(&mut self, scene: SceneId) -> Result<(), AttachmentError> {
        if !self.places.contains_key(&scene) {
            return Ok(());
        }
        self.bump_public()?;
        Arc::make_mut(&mut self.places).remove(&scene);
        Ok(())
    }

    /// Labels the state `machine` is in, for the view's `state_label`.
    pub fn set_state_label(
        &mut self,
        machine: MachineId,
        label: NonEmptyText,
    ) -> Result<(), AttachmentError> {
        within(label.as_str())?;
        if self.state_labels.len() >= self.limits.state_labels
            && !self.state_labels.contains_key(&machine)
        {
            return Err(AttachmentError::TooManyMachines);
        }
        self.bump_public()?;
        Arc::make_mut(&mut self.state_labels).insert(machine, label);
        Ok(())
    }

    /// Appends a line the whole table may see.
    pub fn push_public(&mut self, line: StoryLine) -> Result<(), AttachmentError> {
        line.check()?;
        self.bump_public()?;
        let sequence = self.next_sequence()?;
        push_bounded(
            &mut self.public_story,
            StoryEntry { sequence, line },
            self.limits.story_entries,
        );
        Ok(())
    }

    /// Appends a line only `seat` may see.
    pub fn push_private(&mut self, seat: SeatId, line: StoryLine) -> Result<(), AttachmentError> {
        line.check()?;
        check_room(&self.seat_story, seat, self.limits.seats)?;
        self.bump_seat(seat)?;
        let sequence = self.next_sequence()?;
        let queue = Arc::make_mut(&mut self.seat_story).entry(seat).or_default();
        push_bounded(
            queue,
            StoryEntry { sequence, line },
            self.limits.story_entries,
        );
        Ok(())
    }

    /// A fact only `seat` knows. Setting a fact again replaces its text and makes it the newest.
    pub fn set_private_fact(
        &mut self,
        seat: SeatId,
        fact: FactId,
        text: Secret<String>,
    ) -> Result<(), AttachmentError> {
        within(text.expose())?;
        check_room(&self.seat_facts, seat, self.limits.seats)?;
        let known = self.seat_facts.get(&seat);
        let count = known.map_or(0, |facts| facts.len());
        let has_it = known.is_some_and(|facts| facts.by_id.contains_key(&fact));
        if count >= self.limits.facts_per_seat && !has_it {
            return Err(AttachmentError::TooManyFacts);
        }
        self.bump_seat(seat)?;
        let sequence = self.next_sequence()?;
        let facts = Arc::make_mut(&mut self.seat_facts).entry(seat).or_default();
        Arc::make_mut(facts).set(fact, sequence, text);
        Ok(())
    }

    /// Removes a fact `seat` no longer holds, so its note stops showing it.
    pub fn remove_private_fact(
        &mut self,
        seat: SeatId,
        fact: &FactId,
    ) -> Result<(), AttachmentError> {
        let known = self
            .seat_facts
            .get(&seat)
            .is_some_and(|facts| facts.by_id.contains_key(fact));
        if !known {
            return Ok(());
        }
        self.bump_seat(seat)?;
        if let Some(facts) = Arc::make_mut(&mut self.seat_facts).get_mut(&seat) {
            Arc::make_mut(facts).remove(fact);
        }
        Ok(())
    }

    /// What the active machine says to the whole table, which stands where there is no story yet.
    pub fn set_shared_readout(&mut self, readout: NonEmptyText) -> Result<(), AttachmentError> {
        within(readout.as_str())?;
        self.bump_public()?;
        self.shared_readout = Some(readout);
        Ok(())
    }

    pub fn clear_shared_readout(&mut self) -> Result<(), AttachmentError> {
        if self.shared_readout.is_none() {
            return Ok(());
        }
        self.bump_public()?;
        self.shared_readout = None;
        Ok(())
    }

    /// What the active machine says to one seat.
    pub fn set_seat_readout(
        &mut self,
        seat: SeatId,
        readout: NonEmptyText,
    ) -> Result<(), AttachmentError> {
        within(readout.as_str())?;
        check_room(&self.seat_readouts, seat, self.limits.seats)?;
        self.bump_seat(seat)?;
        Arc::make_mut(&mut self.seat_readouts).insert(seat, readout);
        Ok(())
    }

    pub fn clear_seat_readout(&mut self, seat: SeatId) -> Result<(), AttachmentError> {
        if !self.seat_readouts.contains_key(&seat) {
            return Ok(());
        }
        self.bump_seat(seat)?;
        Arc::make_mut(&mut self.seat_readouts).remove(&seat);
        Ok(())
    }

    /// The offers `seat` may take now, replacing the ones it had. Their text together must fit
    /// `OFFERS_BUDGET_BYTES`, because an offer is never dropped from a view.
    pub fn set_seat_offers(
        &mut self,
        seat: SeatId,
        offers: Vec<OfferEntry>,
    ) -> Result<(), AttachmentError> {
        if offers.len() > self.limits.offers_per_seat {
            return Err(AttachmentError::TooManyOffers);
        }
        let bytes: usize = offers.iter().map(OfferEntry::text_bytes).sum();
        if bytes > OFFERS_BUDGET_BYTES
            || offers
                .iter()
                .any(|offer| offer.text_bytes() > MAXIMUM_TEXT_BYTES)
        {
            return Err(AttachmentError::TextTooLong);
        }
        let mut ids = std::collections::BTreeSet::new();
        if !offers
            .iter()
            .all(|offer| ids.insert(offer.offer_id.as_str()))
        {
            return Err(AttachmentError::DuplicateOffer);
        }
        check_room(&self.seat_offers, seat, self.limits.seats)?;
        self.bump_seat(seat)?;
        Arc::make_mut(&mut self.seat_offers).insert(seat, Arc::new(offers));
        Ok(())
    }

    pub fn clear_seat_offers(&mut self, seat: SeatId) -> Result<(), AttachmentError> {
        if !self.seat_offers.contains_key(&seat) {
            return Ok(());
        }
        self.bump_seat(seat)?;
        Arc::make_mut(&mut self.seat_offers).remove(&seat);
        Ok(())
    }

    /// The decline `seat` got for an input that arrived when the table was at `at`. Its own view
    /// shows it while the table is still at `at` and no other view ever shows it. The reason and
    /// the alternatives must fit the wire frame, like every published text.
    pub fn set_notice(
        &mut self,
        seat: SeatId,
        decline: Decline,
        at: SessionRevision,
    ) -> Result<(), AttachmentError> {
        within(decline.reason())?;
        let alternatives: usize = decline.alternatives().iter().map(String::len).sum();
        if alternatives > OFFERS_BUDGET_BYTES
            || decline
                .alternatives()
                .iter()
                .any(|text| text.len() > MAXIMUM_TEXT_BYTES)
        {
            return Err(AttachmentError::TextTooLong);
        }
        check_room(&self.notices, seat, self.limits.seats)?;
        self.bump_seat(seat)?;
        Arc::make_mut(&mut self.notices).insert(seat, Notice { decline, at });
        Ok(())
    }

    pub fn clear_notice(&mut self, seat: SeatId) -> Result<(), AttachmentError> {
        if !self.notices.contains_key(&seat) {
            return Ok(());
        }
        self.bump_seat(seat)?;
        Arc::make_mut(&mut self.notices).remove(&seat);
        Ok(())
    }

    /// Frees every notice that answered an input before `current`: the table has moved on, so no
    /// view shows them any more.
    pub fn expire_notices(&mut self, current: SessionRevision) -> Result<(), AttachmentError> {
        if !self.notices.values().any(|notice| notice.at < current) {
            return Ok(());
        }
        let expired: Vec<SeatId> = self
            .notices
            .iter()
            .filter(|(_, notice)| notice.at < current)
            .map(|(seat, _)| *seat)
            .collect();
        for seat in expired {
            self.bump_seat(seat)?;
        }
        Arc::make_mut(&mut self.notices).retain(|_, notice| notice.at >= current);
        Ok(())
    }

    /// Wipes everything private to a seat that left the table: its story, facts, offers, readout
    /// and notice (RES36). Call it in the commit that applies `SeatLeft`.
    pub fn forget_seat(&mut self, seat: SeatId) -> Result<(), AttachmentError> {
        let held = self.seat_story.contains_key(&seat)
            || self.seat_facts.contains_key(&seat)
            || self.seat_offers.contains_key(&seat)
            || self.seat_readouts.contains_key(&seat)
            || self.notices.contains_key(&seat);
        if !held {
            return Ok(());
        }
        Arc::make_mut(&mut self.seat_versions).remove(&seat);
        Arc::make_mut(&mut self.seat_story).remove(&seat);
        Arc::make_mut(&mut self.seat_facts).remove(&seat);
        Arc::make_mut(&mut self.seat_offers).remove(&seat);
        Arc::make_mut(&mut self.seat_readouts).remove(&seat);
        Arc::make_mut(&mut self.notices).remove(&seat);
        Ok(())
    }

    /// What the pipeline is doing now (reading, rolling, telling, a picture late). The table's
    /// pause outranks it.
    pub fn set_activity(&mut self, activity: ProjectionStatus) -> Result<(), AttachmentError> {
        if let Some(reason) = activity.reason() {
            within(reason.as_str())?;
        }
        self.bump_public()?;
        self.activity = activity;
        Ok(())
    }

    pub(super) fn place(&self, scene: SceneId) -> Option<&NonEmptyText> {
        self.places.get(&scene)
    }

    pub(super) fn state_label(&self, machine: &MachineId) -> Option<&NonEmptyText> {
        self.state_labels.get(machine)
    }

    pub(super) fn public_story(&self) -> &VecDeque<Arc<StoryEntry>> {
        &self.public_story
    }

    pub(super) fn seat_story(&self, seat: SeatId) -> Option<&VecDeque<Arc<StoryEntry>>> {
        self.seat_story.get(&seat).map(|queue| &**queue)
    }

    pub(super) fn seat_facts(&self, seat: SeatId) -> Option<&SeatFacts> {
        self.seat_facts.get(&seat).map(|facts| &**facts)
    }

    pub(super) fn seat_offers(&self, seat: SeatId) -> &[OfferEntry] {
        self.seat_offers
            .get(&seat)
            .map_or(&[], |offers| offers.as_slice())
    }

    pub(super) fn shared_readout(&self) -> Option<&NonEmptyText> {
        self.shared_readout.as_ref()
    }

    pub(super) fn seat_readout(&self, seat: SeatId) -> Option<&NonEmptyText> {
        self.seat_readouts.get(&seat)
    }

    /// The decline `seat` is shown now: only one that answers the table's current revision.
    pub(crate) fn notice(&self, seat: SeatId, current: SessionRevision) -> Option<&Decline> {
        self.notices
            .get(&seat)
            .filter(|notice| notice.at == current)
            .map(|notice| &notice.decline)
    }

    pub(super) fn activity(&self) -> &ProjectionStatus {
        &self.activity
    }

    #[cfg(test)]
    pub(crate) fn shares_public_story_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.public_story, &other.public_story)
    }

    #[cfg(test)]
    pub(crate) fn shares_seat_facts_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.seat_facts, &other.seat_facts)
    }

    #[cfg(test)]
    pub(crate) fn shares_places_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.places, &other.places)
    }

    #[cfg(test)]
    pub(crate) fn shares_notices_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.notices, &other.notices)
    }
}
