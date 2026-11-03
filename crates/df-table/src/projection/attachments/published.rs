//! The saved form and the digest of what the pipeline publishes to the views.
//!
//! The story, the readouts, the notices, the places, the state labels, the activity and the
//! counters that order them are state that every turn writes and that shows on the shared screen
//! and on every phone, so a table that restarts without them shows blank views until the next turn
//! (#1734) and two tables with equal digests could show different screens. `PublishedRecord` is
//! their saved form (versioned, checked on load, refused with a typed error) and
//! `Attachments::digest` covers them: the story through a digest kept up to date where a line
//! enters or leaves (so it costs the same however long the story is), everything else, which is
//! bounded by the seats, places and machines, by a walk.
//!
//! Two things are left out on purpose. The facts a seat knows belong to the knowledge owner, which
//! publishes them again after a restart, and a seat's offers are derived: the pipeline rebuilds
//! them from its pendings and clarifications when it loads a table. The record holds the texts a
//! seat was shown, so it belongs in the table's own checkpoint and never in a log; its `Debug`
//! shows counts only.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{AttachmentError, Attachments, Notice, OfferEntry, StoryEntry, StoryLine, within};
use crate::identity::{EvidenceRef, MachineId, SceneId, SeatId, SessionRevision};
use crate::outcome::{Decline, DeclineError, DeclineRecord};
use crate::pending::digest::{Fnv, mix};
use crate::private_data::Narration;
use crate::projection::status::ProjectionStatus;
use crate::projection::text::{BlankText, NonEmptyText};
use crate::table_state::{DisplayName, DisplayNameError};

/// The schema version of `PublishedRecord`.
pub const PUBLISHED_RECORD_VERSION: u32 = 1;

/// One story line as saved.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredLine {
    sequence: u64,
    speaker: Option<String>,
    readout: String,
    narration: Option<String>,
    evidence: Option<EvidenceRef>,
}

/// A decline a seat was shown, with the revision it answers.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredNotice {
    seat: u64,
    decline: DeclineRecord,
    at: SessionRevision,
}

/// What the table says it is doing, as saved.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredActivity {
    Ok,
    Reading,
    Rolling,
    Telling,
    ArtDelayed,
    Degraded,
    ReadOnly,
    Reconnecting,
    WaitingFor { who: String, what: String },
}

/// The attachments the pipeline publishes, as saved in a checkpoint.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedRecord {
    schema_version: u32,
    sequence: u64,
    public_version: u64,
    seat_versions: Vec<(u64, u64)>,
    places: Vec<(u64, String)>,
    state_labels: Vec<(MachineId, String)>,
    public_story: Vec<StoredLine>,
    seat_story: Vec<(u64, Vec<StoredLine>)>,
    shared_readout: Option<String>,
    seat_readouts: Vec<(u64, String)>,
    notices: Vec<StoredNotice>,
    activity: StoredActivity,
}

impl fmt::Debug for PublishedRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublishedRecord")
            .field("schema_version", &self.schema_version)
            .field("public_story", &self.public_story.len())
            .field("seat_stories", &self.seat_story.len())
            .field("notices", &self.notices.len())
            .finish_non_exhaustive()
    }
}

/// Why a saved record of the attachments was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishedLoadError {
    UnsupportedVersion {
        found: u32,
        expected: u32,
    },
    /// A text that must not be blank was.
    BlankText,
    /// A name that cannot be shown.
    InvalidName,
    /// A decline that cannot be built.
    InvalidDecline,
    /// The same seat, scene or machine twice.
    Duplicate,
    /// Story lines out of order, or a sequence beyond the counter.
    StoryOutOfOrder,
}

impl fmt::Display for PublishedLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { found, expected } => write!(
                formatter,
                "published views at version {found} were refused; this engine reads {expected}"
            ),
            Self::BlankText => formatter.write_str("a saved view text was blank"),
            Self::InvalidName => formatter.write_str("a saved name cannot be shown"),
            Self::InvalidDecline => formatter.write_str("a saved decline cannot be built"),
            Self::Duplicate => formatter.write_str("a saved view holds the same entry twice"),
            Self::StoryOutOfOrder => formatter.write_str("saved story lines are out of order"),
        }
    }
}

impl std::error::Error for PublishedLoadError {}

impl From<BlankText> for PublishedLoadError {
    fn from(_: BlankText) -> Self {
        Self::BlankText
    }
}

impl From<DisplayNameError> for PublishedLoadError {
    fn from(_: DisplayNameError) -> Self {
        Self::InvalidName
    }
}

impl From<DeclineError> for PublishedLoadError {
    fn from(_: DeclineError) -> Self {
        Self::InvalidDecline
    }
}

/// The hash one story entry contributes to the story digest: which queue it is in (0 for the public
/// one, the seat plus one for a private one), its place in the story and its content.
pub(super) fn entry_hash(queue: u64, entry: &StoryEntry) -> u64 {
    let mut hasher = Fnv::new();
    queue.hash(&mut hasher);
    entry.sequence.hash(&mut hasher);
    entry
        .line
        .speaker
        .as_ref()
        .map(DisplayName::as_str)
        .hash(&mut hasher);
    entry.line.readout.as_str().hash(&mut hasher);
    entry
        .line
        .narration
        .as_ref()
        .map(Narration::expose)
        .hash(&mut hasher);
    entry.line.evidence.hash(&mut hasher);
    mix(hasher.finish())
}

/// The part of the story digest one whole queue contributes.
pub(super) fn queue_hash(queue: u64, entries: &VecDeque<Arc<StoryEntry>>) -> u64 {
    entries
        .iter()
        .fold(0, |folded, entry| folded ^ entry_hash(queue, entry))
}

fn stored_line(entry: &StoryEntry) -> StoredLine {
    StoredLine {
        sequence: entry.sequence,
        speaker: entry
            .line
            .speaker
            .as_ref()
            .map(|name| name.as_str().to_owned()),
        readout: entry.line.readout.as_str().to_owned(),
        narration: entry
            .line
            .narration
            .as_ref()
            .map(|text| text.expose().to_owned()),
        evidence: entry.line.evidence,
    }
}

fn loaded_line(stored: &StoredLine) -> Result<StoryEntry, PublishedLoadError> {
    let mut line = StoryLine::new(NonEmptyText::new(&stored.readout)?);
    if let Some(speaker) = &stored.speaker {
        line = line.spoken_by(DisplayName::new(speaker)?);
    }
    if let Some(narration) = &stored.narration {
        line = line.with_narration(Narration::new(narration.clone()));
    }
    if let Some(evidence) = stored.evidence {
        line = line.with_evidence(evidence);
    }
    Ok(StoryEntry {
        sequence: stored.sequence,
        line,
    })
}

fn loaded_queue(
    lines: &[StoredLine],
    sequence: u64,
) -> Result<VecDeque<Arc<StoryEntry>>, PublishedLoadError> {
    let mut queue = VecDeque::with_capacity(lines.len());
    let mut last = 0;
    for stored in lines {
        if stored.sequence <= last || stored.sequence > sequence {
            return Err(PublishedLoadError::StoryOutOfOrder);
        }
        last = stored.sequence;
        queue.push_back(Arc::new(loaded_line(stored)?));
    }
    Ok(queue)
}

fn stored_activity(activity: &ProjectionStatus) -> StoredActivity {
    match activity {
        ProjectionStatus::Ok => StoredActivity::Ok,
        ProjectionStatus::Reading => StoredActivity::Reading,
        ProjectionStatus::Rolling => StoredActivity::Rolling,
        ProjectionStatus::Telling => StoredActivity::Telling,
        ProjectionStatus::ArtDelayed => StoredActivity::ArtDelayed,
        ProjectionStatus::Degraded => StoredActivity::Degraded,
        ProjectionStatus::ReadOnly => StoredActivity::ReadOnly,
        ProjectionStatus::Reconnecting => StoredActivity::Reconnecting,
        ProjectionStatus::WaitingFor { who, what } => StoredActivity::WaitingFor {
            who: who.as_str().to_owned(),
            what: what.as_str().to_owned(),
        },
    }
}

fn loaded_activity(stored: &StoredActivity) -> Result<ProjectionStatus, PublishedLoadError> {
    Ok(match stored {
        StoredActivity::Ok => ProjectionStatus::Ok,
        StoredActivity::Reading => ProjectionStatus::Reading,
        StoredActivity::Rolling => ProjectionStatus::Rolling,
        StoredActivity::Telling => ProjectionStatus::Telling,
        StoredActivity::ArtDelayed => ProjectionStatus::ArtDelayed,
        StoredActivity::Degraded => ProjectionStatus::Degraded,
        StoredActivity::ReadOnly => ProjectionStatus::ReadOnly,
        StoredActivity::Reconnecting => ProjectionStatus::Reconnecting,
        StoredActivity::WaitingFor { who, what } => ProjectionStatus::waiting_for(who, what)?,
    })
}

/// Fails on the second use of a key, which a saved record never has.
fn unique<K: Ord, V>(
    entries: impl IntoIterator<Item = (K, V)>,
) -> Result<BTreeMap<K, V>, PublishedLoadError> {
    let mut map = BTreeMap::new();
    for (key, value) in entries {
        if map.insert(key, value).is_some() {
            return Err(PublishedLoadError::Duplicate);
        }
    }
    Ok(map)
}

impl Attachments {
    /// Gives the public line at `sequence` the storyteller's words, keeping the readout it has.
    /// Returns whether the line is still in the story: a line the story dropped is not brought
    /// back, and a line that already has narration keeps the first.
    pub fn extend_public_narration(
        &mut self,
        sequence: u64,
        narration: Narration,
    ) -> Result<bool, AttachmentError> {
        within(narration.expose())?;
        let Ok(position) = self
            .public_story
            .binary_search_by_key(&sequence, |entry| entry.sequence)
        else {
            return Ok(false);
        };
        if self.public_story[position].line.narration.is_some() {
            return Ok(true);
        }
        self.bump_public()?;
        let queue = Arc::make_mut(&mut self.public_story);
        let old = Arc::clone(&queue[position]);
        let mut line = old.line.clone();
        line.narration = Some(narration);
        let renewed = StoryEntry {
            sequence: old.sequence,
            line,
        };
        self.story_digest ^= entry_hash(0, &old) ^ entry_hash(0, &renewed);
        queue[position] = Arc::new(renewed);
        Ok(true)
    }

    /// Puts back the offers a seat held when the table was saved, without raising its attachment
    /// version: the saved version already counts the publish that made them, so a restart changes
    /// nothing a watcher can see.
    pub fn restore_seat_offers(
        &mut self,
        seat: SeatId,
        offers: Vec<OfferEntry>,
    ) -> Result<(), AttachmentError> {
        let versions = Arc::clone(&self.seat_versions);
        self.set_seat_offers(seat, offers)?;
        self.seat_versions = versions;
        Ok(())
    }

    /// What the pipeline published, in its saved form.
    pub fn to_published(&self) -> PublishedRecord {
        PublishedRecord {
            schema_version: PUBLISHED_RECORD_VERSION,
            sequence: self.sequence,
            public_version: self.public_version,
            seat_versions: self
                .seat_versions
                .iter()
                .map(|(seat, count)| (seat.get(), *count))
                .collect(),
            places: self
                .places
                .iter()
                .map(|(scene, place)| (scene.get(), place.as_str().to_owned()))
                .collect(),
            state_labels: self
                .state_labels
                .iter()
                .map(|(machine, label)| (machine.clone(), label.as_str().to_owned()))
                .collect(),
            public_story: self
                .public_story
                .iter()
                .map(|entry| stored_line(entry))
                .collect(),
            seat_story: self
                .seat_story
                .iter()
                .map(|(seat, queue)| {
                    (
                        seat.get(),
                        queue.iter().map(|entry| stored_line(entry)).collect(),
                    )
                })
                .collect(),
            shared_readout: self
                .shared_readout
                .as_ref()
                .map(|text| text.as_str().to_owned()),
            seat_readouts: self
                .seat_readouts
                .iter()
                .map(|(seat, text)| (seat.get(), text.as_str().to_owned()))
                .collect(),
            notices: self
                .notices
                .iter()
                .map(|(seat, notice)| StoredNotice {
                    seat: seat.get(),
                    decline: notice.decline.to_record(),
                    at: notice.at,
                })
                .collect(),
            activity: stored_activity(&self.activity),
        }
    }

    /// The attachments of a restored process: what the pipeline had published, with the story's
    /// order and the counters as they were. A record this engine cannot read is refused and
    /// nothing is half loaded.
    pub fn from_published(record: &PublishedRecord) -> Result<Self, PublishedLoadError> {
        if record.schema_version != PUBLISHED_RECORD_VERSION {
            return Err(PublishedLoadError::UnsupportedVersion {
                found: record.schema_version,
                expected: PUBLISHED_RECORD_VERSION,
            });
        }
        let mut attachments = Self::new();
        attachments.sequence = record.sequence;
        attachments.public_version = record.public_version;
        attachments.seat_versions = Arc::new(unique(
            record
                .seat_versions
                .iter()
                .map(|(seat, count)| (SeatId::new(*seat), *count)),
        )?);
        attachments.places = Arc::new(unique(
            record
                .places
                .iter()
                .map(|(scene, text)| {
                    NonEmptyText::new(text).map(|place| (SceneId::new(*scene), place))
                })
                .collect::<Result<Vec<_>, _>>()?,
        )?);
        attachments.state_labels = Arc::new(unique(
            record
                .state_labels
                .iter()
                .map(|(machine, text)| {
                    NonEmptyText::new(text).map(|label| (machine.clone(), label))
                })
                .collect::<Result<Vec<_>, _>>()?,
        )?);
        let public = loaded_queue(&record.public_story, record.sequence)?;
        attachments.story_digest ^= queue_hash(0, &public);
        attachments.public_story = Arc::new(public);
        let mut private = BTreeMap::new();
        for (seat, lines) in &record.seat_story {
            let queue = loaded_queue(lines, record.sequence)?;
            attachments.story_digest ^= queue_hash(seat.saturating_add(1), &queue);
            if private
                .insert(SeatId::new(*seat), Arc::new(queue))
                .is_some()
            {
                return Err(PublishedLoadError::Duplicate);
            }
        }
        attachments.seat_story = Arc::new(private);
        attachments.shared_readout = record
            .shared_readout
            .as_deref()
            .map(NonEmptyText::new)
            .transpose()?;
        attachments.seat_readouts = Arc::new(unique(
            record
                .seat_readouts
                .iter()
                .map(|(seat, text)| NonEmptyText::new(text).map(|t| (SeatId::new(*seat), t)))
                .collect::<Result<Vec<_>, _>>()?,
        )?);
        attachments.notices = Arc::new(unique(
            record
                .notices
                .iter()
                .map(|stored| {
                    Decline::from_record(stored.decline.clone()).map(|decline| {
                        (
                            SeatId::new(stored.seat),
                            Notice {
                                decline,
                                at: stored.at,
                            },
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        )?);
        attachments.activity = loaded_activity(&record.activity)?;
        Ok(attachments)
    }

    /// A digest of everything the views are built from except the facts a seat knows: the same
    /// attachments give the same digest in every run, and a story line, a readout, a notice, an
    /// offer or the activity that differs gives another. The story is covered by the digest kept
    /// where lines enter and leave; the rest is a walk over collections bounded by the seats,
    /// places and machines.
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv::new();
        self.story_digest.hash(&mut hasher);
        self.sequence.hash(&mut hasher);
        self.public_version.hash(&mut hasher);
        for (seat, count) in self.seat_versions.iter() {
            (seat.get(), *count).hash(&mut hasher);
        }
        for (scene, place) in self.places.iter() {
            (scene.get(), place.as_str()).hash(&mut hasher);
        }
        for (machine, label) in self.state_labels.iter() {
            (machine.as_str(), label.as_str()).hash(&mut hasher);
        }
        self.shared_readout
            .as_ref()
            .map(NonEmptyText::as_str)
            .hash(&mut hasher);
        for (seat, text) in self.seat_readouts.iter() {
            (seat.get(), text.as_str()).hash(&mut hasher);
        }
        for (seat, notice) in self.notices.iter() {
            seat.get().hash(&mut hasher);
            notice.decline.code().hash(&mut hasher);
            notice.decline.reason().hash(&mut hasher);
            notice.decline.missing_piece().hash(&mut hasher);
            notice.decline.alternatives().hash(&mut hasher);
            // `at` is the revision of a commit, a stamp the host moves: the digest leaves it out,
            // like every stamp (S1), and a notice is shown only while the table is still at it.
        }
        for (seat, offers) in self.seat_offers.iter() {
            seat.get().hash(&mut hasher);
            for offer in offers.iter() {
                offer.offer_id.as_str().hash(&mut hasher);
                offer.label.as_str().hash(&mut hasher);
                offer
                    .explain
                    .as_ref()
                    .map(NonEmptyText::as_str)
                    .hash(&mut hasher);
                offer
                    .group
                    .as_ref()
                    .map(NonEmptyText::as_str)
                    .hash(&mut hasher);
                offer
                    .unavailable_reason
                    .as_ref()
                    .map(NonEmptyText::as_str)
                    .hash(&mut hasher);
            }
        }
        stored_activity_key(&self.activity).hash(&mut hasher);
        hasher.finish()
    }
}

/// The activity as a hashable value: its kind and, for a wait, the words.
fn stored_activity_key(activity: &ProjectionStatus) -> (&'static str, &str, &str) {
    match activity {
        ProjectionStatus::WaitingFor { who, what } => ("waiting_for", who.as_str(), what.as_str()),
        other => (other.kind().key(), "", ""),
    }
}
