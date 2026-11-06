//! Stored rulings and their keyed reuse (K.4, RES48).
//!
//! It owns `ScopedRuling` (an answer, the question's typed key and scope, who decided it and the
//! action that asked) and `RulingBook`, the bounded, indexed set of rulings the table has made. A
//! later request is matched by equality of the key and scope alone, in one map lookup; a
//! retrieval or similarity hit is never a match, and the match never reads player words. A reuse
//! that grants advantage or automatic success is confirmed once per scene
//! (`needs_reconfirmation`). The host can revoke a ruling. It never decides an answer and never
//! touches a die: the rules owner validates an answer, and the host or a reviewed automatic
//! policy gives it.
//!
//! The book holds at most its maximum and forgets the oldest first when a newer one arrives, so
//! memory stays bounded however long play lasts. It keeps an exclusive-or digest of its rulings,
//! updated where one changes. The saved form (`RulingRecord`) is versioned and checked on load: a
//! newer record, a maximum of zero, more rulings than the maximum or the same question twice is
//! refused with a typed `RulingLoadError`, never loaded silently.

use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use super::bounded::BoundedMap;
use super::digest::{Fnv, mix};
use crate::identity::SceneId;
use crate::limits::MAXIMUM_STORED_RULINGS;
use crate::listed::listed_enum;
use crate::ports::{ActionId, RulingAnswer, RulingKey, RulingScope, RulingTier};

/// The schema version of `RulingRecord`.
pub(crate) const RULING_SCHEMA_VERSION: u32 = 1;

listed_enum! {
    /// Who made a ruling.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum RulingSource {
        /// The host's one tap on the suggestion, an alternative or a refusal.
        Host,
        /// A reviewed, source-grounded automatic adjudication (RES15).
        ReviewedAutomatic,
    }
}

impl RulingSource {
    pub fn key(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::ReviewedAutomatic => "reviewed_automatic",
        }
    }
}

/// One ruling the table has made.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedRuling {
    key: RulingKey,
    /// The scene a `Scene` ruling binds to; `None` for a table ruling.
    scene: Option<SceneId>,
    answer: RulingAnswer,
    tier: RulingTier,
    source: RulingSource,
    /// The action that asked the question, kept beside the key (C.3).
    asked_by_action: ActionId,
    /// The scene in which a reuse was last confirmed, when the tier needs it.
    reconfirmed_in: Option<SceneId>,
}

impl ScopedRuling {
    /// A ruling made in `scene_now` for a question of `scope`.
    pub fn new(
        key: RulingKey,
        scope: RulingScope,
        scene_now: Option<SceneId>,
        answer: RulingAnswer,
        tier: RulingTier,
        source: RulingSource,
        asked_by_action: ActionId,
    ) -> Self {
        let scene = match scope {
            RulingScope::Table => None,
            RulingScope::Scene => scene_now,
        };
        Self {
            key,
            scene,
            answer,
            tier,
            source,
            asked_by_action,
            reconfirmed_in: scene_now,
        }
    }

    pub fn key(&self) -> &RulingKey {
        &self.key
    }

    pub fn answer(&self) -> &RulingAnswer {
        &self.answer
    }

    pub fn tier(&self) -> RulingTier {
        self.tier
    }

    pub fn source(&self) -> RulingSource {
        self.source
    }

    pub fn asked_by_action(&self) -> &ActionId {
        &self.asked_by_action
    }

    /// Whether reusing this ruling in `scene_now` must first be confirmed again (RES48).
    pub fn needs_reconfirmation(&self, scene_now: Option<SceneId>) -> bool {
        self.tier.needs_reconfirmation() && self.reconfirmed_in != scene_now
    }

    fn slot(&self) -> RulingSlot {
        (self.key.clone(), self.scene)
    }

    fn logical_hash(&self) -> u64 {
        let mut hasher = Fnv::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

/// What a ruling is stored under: the question's key and the scene it binds to, if any.
type RulingSlot = (RulingKey, Option<SceneId>);

/// The slot a question looks in: a table ruling has no scene, a scene ruling its own.
fn slot_of(key: &RulingKey, scope: RulingScope, scene_now: Option<SceneId>) -> Option<RulingSlot> {
    match scope {
        RulingScope::Table => Some((key.clone(), None)),
        RulingScope::Scene => scene_now.map(|scene| (key.clone(), Some(scene))),
    }
}

/// Why a saved ruling record was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RulingLoadError {
    UnsupportedVersion {
        found: u32,
        expected: u32,
    },
    /// A book that holds nothing cannot hold what the record holds.
    MaximumIsZero,
    TooManyForMaximum,
    /// The same question has two rulings.
    DuplicateQuestion,
}

impl fmt::Display for RulingLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { found, expected } => write!(
                formatter,
                "a ruling record at version {found} was refused; this engine reads {expected}"
            ),
            Self::MaximumIsZero => {
                formatter.write_str("a ruling record needs a maximum above zero")
            }
            Self::TooManyForMaximum => {
                formatter.write_str("a ruling record holds more rulings than its maximum")
            }
            Self::DuplicateQuestion => {
                formatter.write_str("a ruling record answers a question twice")
            }
        }
    }
}

impl std::error::Error for RulingLoadError {}

/// The saved form of a `RulingBook`, oldest ruling first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulingRecord {
    schema_version: u32,
    maximum: usize,
    entries: Vec<ScopedRuling>,
}

/// The rulings a table has made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulingBook {
    entries: BoundedMap<RulingSlot, ScopedRuling>,
    digest: u64,
}

impl Default for RulingBook {
    fn default() -> Self {
        Self::remembering(MAXIMUM_STORED_RULINGS)
    }
}

impl RulingBook {
    /// A book that keeps at most `maximum` rulings.
    pub fn remembering(maximum: usize) -> Self {
        Self {
            entries: BoundedMap::holding_at_most(maximum),
            digest: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.len() == 0
    }

    /// A digest of the logical state: every ruling and the bound. Kept up to date where a ruling
    /// changes, so asking costs nothing.
    pub fn digest(&self) -> u64 {
        self.digest ^ mix(self.entries.maximum() as u64)
    }

    /// The stored ruling for this question, if the key and scope are equal.
    pub fn find(
        &self,
        key: &RulingKey,
        scope: RulingScope,
        scene_now: Option<SceneId>,
    ) -> Option<&ScopedRuling> {
        let slot = slot_of(key, scope, scene_now)?;
        self.entries.get(&slot)
    }

    /// Stores a ruling, replacing one for the same question, and forgets the oldest ruling beyond
    /// the maximum.
    pub fn store(&mut self, ruling: ScopedRuling) {
        self.digest ^= ruling.logical_hash();
        let removed = self.entries.insert(ruling.slot(), ruling);
        for old in removed.replaced.iter().chain(removed.evicted.iter()) {
            self.digest ^= old.logical_hash();
        }
    }

    /// Records that a reuse was confirmed in `scene_now`.
    pub fn reconfirm(&mut self, key: &RulingKey, scope: RulingScope, scene_now: Option<SceneId>) {
        let Some(slot) = slot_of(key, scope, scene_now) else {
            return;
        };
        if let Some(ruling) = self.entries.get_mut(&slot) {
            self.digest ^= ruling.logical_hash();
            ruling.reconfirmed_in = scene_now;
            self.digest ^= ruling.logical_hash();
        }
    }

    /// Takes a ruling back (the host's revoke, RES48); whether one was stored.
    pub fn revoke(
        &mut self,
        key: &RulingKey,
        scope: RulingScope,
        scene_now: Option<SceneId>,
    ) -> bool {
        let Some(slot) = slot_of(key, scope, scene_now) else {
            return false;
        };
        match self.entries.remove(&slot) {
            Some(old) => {
                self.digest ^= old.logical_hash();
                true
            }
            None => false,
        }
    }

    /// The saved form, oldest ruling first.
    pub fn to_record(&self) -> RulingRecord {
        RulingRecord {
            schema_version: RULING_SCHEMA_VERSION,
            maximum: self.entries.maximum(),
            entries: self.entries.oldest_first().into_iter().cloned().collect(),
        }
    }

    /// A book from a saved record, refusing one that is newer than this engine, has a maximum of
    /// zero, holds more than its maximum or answers a question twice.
    pub fn from_record(record: RulingRecord) -> Result<Self, RulingLoadError> {
        if record.schema_version != RULING_SCHEMA_VERSION {
            return Err(RulingLoadError::UnsupportedVersion {
                found: record.schema_version,
                expected: RULING_SCHEMA_VERSION,
            });
        }
        if record.maximum == 0 {
            return Err(RulingLoadError::MaximumIsZero);
        }
        if record.entries.len() > record.maximum {
            return Err(RulingLoadError::TooManyForMaximum);
        }
        let mut book = Self::remembering(record.maximum);
        for ruling in record.entries {
            if book.entries.contains(&ruling.slot()) {
                return Err(RulingLoadError::DuplicateQuestion);
            }
            book.store(ruling);
        }
        Ok(book)
    }

    /// The digest worked out again from the rulings, to prove the kept one is right.
    #[cfg(test)]
    pub(crate) fn recomputed_digest(&self) -> u64 {
        super::digest::walked(self.entries.len());
        let folded = self
            .entries
            .values()
            .fold(0, |folded, ruling| folded ^ ruling.logical_hash());
        folded ^ mix(self.entries.maximum() as u64)
    }
}
