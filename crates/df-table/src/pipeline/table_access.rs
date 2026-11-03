//! The one adapter between the pipeline and the table's state.
//!
//! The pipeline reads and changes the table only through `PipelineTable`, which composes the
//! shared facts and the machine slot that live in `TableState` (the one home of the active slot:
//! the focus scene's machine, else the table machine, RES27) with what only the pipeline holds:
//! the open pending resolutions, the stored rulings, the resource requests that are out, the
//! results of committed operations, each seat's open clarification, the stamp of each seat's last
//! change of legal choices and the story and notices the projections read. When the table state
//! grows to hold more of these, this is the file that changes.
//!
//! Every book is behind an `Arc` and changed with copy on write, and each is a `BoundedMap` that
//! shares its buckets and its order queue, so `checkpoint` is a few thousand pointer copies and the
//! first change after it copies one bucket, not the history: a host that groups inputs into one
//! commit takes a checkpoint before the batch and restores it if the commit is refused (O.16 S1).
//! `state_hash` is a stable digest of the table's facts without the commit stamps, the memory of
//! applied operations (commit ids are not in it) and every book, the published views included: no
//! clock reading, each book's digest kept up to date where an entry changes, and the whole kept
//! until the next change, so an input costs the same however much a book holds and `hash_before`
//! of an input is the `hash_after` of the one before it.
//!
//! It never decides what a turn does: it applies what a stage staged, all or nothing for the
//! table's facts, and answers reads. `to_record` and `from_record` are the saved form of
//! everything the pipeline holds, with the pendings' waits saved as ages.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::PipelineLimits;
use super::books::{
    BookLoadError, ReleaseBook, ReleaseRecord, RequestBook, RequestRecord, ResultBook, ResultRecord,
};
use super::offers::{MachineSurfaceError, OffersError};
use crate::effects::{ApplyOutcome, EffectError, TableEffect, TableState};
use crate::envelope::Sender;
use crate::identity::{Correlation, SceneId, SeatId, SessionRevision, TableId, TableScope};
use crate::machine::MachineId;
use crate::machine_slot::MachineSlot;
use crate::pending::digest::Fnv;
use crate::pending::{
    PendingBook, PendingLoadError, PendingRecord, RulingBook, RulingLoadError, RulingRecord,
};
use crate::ports::ActionId;
use crate::projection::{Attachments, PublishedLoadError, PublishedRecord};
use crate::registry::Registry;
use crate::table_state::{DigestMemo, MachineHome, TableSnapshot};

/// The machine that runs now, borrowed from a snapshot of the table.
#[derive(Debug, Clone, Copy)]
pub(super) struct Running<'s> {
    pub(super) home: MachineHome,
    pub(super) id: &'s MachineId,
    pub(super) slot: &'s MachineSlot,
}

/// The table as the pipeline sees it.
#[derive(Debug, Clone)]
pub struct PipelineTable {
    table: TableId,
    facts: TableState,
    pending: Arc<PendingBook>,
    rulings: Arc<RulingBook>,
    requests: Arc<RequestBook>,
    results: Arc<ResultBook>,
    /// How inputs that committed nothing ended (declined, clarified, routed): kept so a duplicate
    /// delivery is answered without a model call. A cache of the session: it is neither saved nor
    /// in the digest, so a restart may ask the interpreter again, and a batch that only declined
    /// changes nothing worth a checkpoint.
    ends: Arc<ResultBook>,
    releases: Arc<ReleaseBook>,
    attachments: Arc<Attachments>,
    clarifications: Arc<BTreeMap<SeatId, Vec<ActionId>>>,
    choice_stamps: BTreeMap<SeatId, SessionRevision>,
    host: Option<SeatId>,
    hash_memo: DigestMemo,
    durable_memo: DigestMemo,
}

impl PipelineTable {
    /// A table with no machine running yet and nothing pending.
    pub fn new(table: TableId, facts: TableState) -> Self {
        Self::with_limits(table, facts, &PipelineLimits::default())
    }

    pub fn with_limits(table: TableId, facts: TableState, limits: &PipelineLimits) -> Self {
        Self {
            table,
            facts,
            pending: Arc::new(PendingBook::holding_at_most(limits.maximum_open_pendings)),
            rulings: Arc::new(RulingBook::remembering(limits.maximum_stored_rulings)),
            requests: Arc::new(RequestBook::holding_at_most(
                limits.maximum_outstanding_requests,
            )),
            results: Arc::new(ResultBook::remembering(limits.maximum_remembered_results)),
            ends: Arc::new(ResultBook::remembering(limits.maximum_remembered_results)),
            releases: Arc::new(ReleaseBook::holding_at_most(
                limits.maximum_outstanding_requests,
            )),
            attachments: Arc::new(Attachments::new()),
            clarifications: Arc::new(BTreeMap::new()),
            choice_stamps: BTreeMap::new(),
            host: None,
            hash_memo: DigestMemo::empty(),
            durable_memo: DigestMemo::empty(),
        }
    }

    /// The seat whose phone adjudicates rulings (K.4).
    pub fn hosted_by(mut self, host: SeatId) -> Self {
        self.host = Some(host);
        self.forget_hashes();
        self
    }

    /// The table with these rulings already stored, as a host that seeds a table does.
    pub fn with_rulings(mut self, rulings: RulingBook) -> Self {
        self.rulings = Arc::new(rulings);
        self.forget_hashes();
        self
    }

    /// Starts the machine of `slot` at `home`, as a committed effect.
    pub fn start_machine(
        &mut self,
        correlation: Correlation,
        home: MachineHome,
        slot: MachineSlot,
    ) -> Result<(), EffectError> {
        let machine = slot.machine().clone();
        let effects = [TableEffect::ActiveMachineSet {
            home,
            machine,
            slot,
        }];
        let scope = self.scope();
        self.forget_hashes();
        self.facts.apply(&scope, correlation, &effects).map(|_| ())
    }

    /// Records that the seat's legal choices changed at `revision`, so an older view is checked
    /// again (C.0 ADMIT). The owner calls it when it commits a change to what a seat may do.
    pub fn stamp_choices_changed(&mut self, seat: SeatId, revision: SessionRevision) {
        self.choice_stamps.insert(seat, revision);
        self.forget_hashes();
    }

    /// Stamps the table's revision with the revision of the commit that held the inputs run so far:
    /// the host calls it once at a batch commit. The pipeline never stamps it from an input's own
    /// correlation, and the commit's id is not remembered as an operation, so a batch of N and N
    /// batches of one end in the same state digest (S1: the digest leaves the stamps out).
    pub fn stamp_commit_revision(&mut self, commit: Correlation) -> Result<(), EffectError> {
        self.facts.stamp_commit(commit.revision);
        self.forget_hashes();
        Ok(())
    }

    /// A copy to restore if a batch commit is refused: the books and the table's memory of applied
    /// operations share their buckets, so it copies pointers and no entry, however much history
    /// they hold.
    pub fn checkpoint(&self) -> Self {
        self.clone()
    }

    pub fn table_id(&self) -> TableId {
        self.table
    }

    /// The shared facts, which a machine reads on every call.
    pub fn facts(&self) -> &TableState {
        &self.facts
    }

    pub fn pending(&self) -> &PendingBook {
        &self.pending
    }

    pub fn rulings(&self) -> &RulingBook {
        &self.rulings
    }

    pub fn requests(&self) -> &RequestBook {
        &self.requests
    }

    pub fn results(&self) -> &ResultBook {
        &self.results
    }

    /// How inputs that committed nothing ended, as far as this session remembers.
    pub fn ends(&self) -> &ResultBook {
        &self.ends
    }

    /// The reservations a cancelled pending gave up and the rules owner has not been told about.
    pub fn releases(&self) -> &ReleaseBook {
        &self.releases
    }

    /// What the projections read: the story so far, the notices, the readouts and the activity.
    pub fn attachments(&self) -> &Attachments {
        &self.attachments
    }

    pub fn host(&self) -> Option<SeatId> {
        self.host
    }

    pub fn active_scene(&self) -> Option<SceneId> {
        self.facts.active_scene()
    }

    pub fn has_seat(&self, seat: SeatId) -> bool {
        self.facts.seats().contains(&seat)
    }

    /// The id of the machine that runs now, if one does.
    pub fn running_machine(&self) -> Option<MachineId> {
        self.facts
            .snapshot()
            .focus_machine()
            .map(|active| active.machine().clone())
    }

    /// The machine that runs now and its saved state, borrowed from `snapshot`: the focus scene's
    /// machine, else the table machine.
    pub(super) fn running<'s>(&self, snapshot: &'s TableSnapshot) -> Option<Running<'s>> {
        let from_scene = snapshot
            .focus_scene()
            .and_then(|scene| Some((scene, snapshot.scene(scene)?.active()?)));
        match (from_scene, snapshot.table_machine()) {
            (Some((scene, active)), _) => Some(Running {
                home: MachineHome::Scene { scene },
                id: active.machine(),
                slot: active.slot(),
            }),
            (None, Some(active)) => Some(Running {
                home: MachineHome::Table,
                id: active.machine(),
                slot: active.slot(),
            }),
            (None, None) => None,
        }
    }

    /// Where an event of this table happened now.
    pub fn scope(&self) -> TableScope {
        let scope = TableScope::of_table(self.table);
        let scope = match self.facts.active_scene() {
            Some(scene) => scope.in_scene(scene),
            None => scope,
        };
        match self.running_machine() {
            Some(machine) => scope.in_machine(machine),
            None => scope,
        }
    }

    /// The choices the seat was last shown as a clarification, if it has one open.
    pub fn clarification_of(&self, seat: SeatId) -> Option<&[ActionId]> {
        self.clarifications.get(&seat).map(Vec::as_slice)
    }

    /// Whether a view drawn at `revision` is older than the seat's last change of legal choices.
    pub fn view_is_stale(&self, seat: SeatId, revision: SessionRevision) -> bool {
        self.choice_stamps
            .get(&seat)
            .is_some_and(|stamp| revision < *stamp)
    }

    pub(super) fn pending_mut(&mut self) -> &mut PendingBook {
        self.forget_hashes();
        Arc::make_mut(&mut self.pending)
    }

    pub(super) fn rulings_mut(&mut self) -> &mut RulingBook {
        self.forget_hashes();
        Arc::make_mut(&mut self.rulings)
    }

    pub(super) fn requests_mut(&mut self) -> &mut RequestBook {
        self.forget_hashes();
        Arc::make_mut(&mut self.requests)
    }

    pub(super) fn results_mut(&mut self) -> &mut ResultBook {
        self.forget_hashes();
        Arc::make_mut(&mut self.results)
    }

    pub(super) fn ends_mut(&mut self) -> &mut ResultBook {
        Arc::make_mut(&mut self.ends)
    }

    pub(super) fn releases_mut(&mut self) -> &mut ReleaseBook {
        self.forget_hashes();
        Arc::make_mut(&mut self.releases)
    }

    pub(super) fn attachments_mut(&mut self) -> &mut Attachments {
        self.hash_memo.forget();
        Arc::make_mut(&mut self.attachments)
    }

    pub(super) fn set_clarification(&mut self, seat: SeatId, options: Vec<ActionId>) {
        self.forget_hashes();
        Arc::make_mut(&mut self.clarifications).insert(seat, options);
    }

    pub(super) fn clear_clarification(&mut self, seat: SeatId) {
        if self.clarifications.contains_key(&seat) {
            self.forget_hashes();
            Arc::make_mut(&mut self.clarifications).remove(&seat);
        }
    }

    /// Forgets the clarification and the choice stamp of every seat that has left the table, so
    /// neither map holds more entries than there are seats.
    pub(super) fn forget_absent_seats(&mut self) {
        let seats = self.facts.seats();
        if self.clarifications.keys().any(|seat| !seats.contains(seat)) {
            self.forget_hashes();
            Arc::make_mut(&mut self.clarifications).retain(|seat, _| seats.contains(seat));
        }
        if self.choice_stamps.keys().any(|seat| !seats.contains(seat)) {
            self.forget_hashes();
            self.choice_stamps.retain(|seat, _| seats.contains(seat));
        }
    }

    /// Forgets both digests: something the pipeline keeps changed.
    fn forget_hashes(&self) {
        self.hash_memo.forget();
        self.durable_memo.forget();
    }

    /// The correlation the table state is given. The table's revision is the commit's, which the
    /// host stamps (`stamp_commit_revision`); a turn carries the table's current revision, or the
    /// start of the input's epoch when that is later, and never the revision of its own admission.
    pub(super) fn state_correlation(&self, correlation: Correlation) -> Correlation {
        let current = self.facts.snapshot().revision();
        let floor = SessionRevision::new(correlation.revision.epoch(), 0);
        Correlation {
            revision: current.max(floor),
            ..correlation
        }
    }

    /// Applies one commit's effects to the shared facts, all or nothing, once per operation id.
    pub(super) fn apply_effects(
        &mut self,
        sender: Sender,
        scope: &TableScope,
        correlation: Correlation,
        effects: &[TableEffect],
    ) -> Result<ApplyOutcome, EffectError> {
        let given = self.state_correlation(correlation);
        self.forget_hashes();
        self.facts.apply_from(sender, scope, given, effects)
    }

    /// A stable digest of everything a turn can change, for golden sessions and replay: the facts
    /// of the table without the commit stamps, the memory of applied operations, each book and the
    /// views the table shows (the story, the readouts, the notices, the offers, the activity). It
    /// reads no clock and depends on no commit id: a batch of N inputs and N batches of one are
    /// equal. It is kept until the table next changes, so asking costs nothing the second time.
    pub fn state_hash(&self) -> u64 {
        if let Some(known) = self.hash_memo.get() {
            return known;
        }
        let mut hasher = Fnv::new();
        self.durable_hash().hash(&mut hasher);
        self.attachments.digest().hash(&mut hasher);
        let worked_out = hasher.finish();
        self.hash_memo.set(worked_out);
        worked_out
    }

    /// The digest of what a checkpoint must keep for the table to go on: the facts without the
    /// commit stamps, the memory of applied operations and every book, and not the views. A batch
    /// that changed only what the views show (a decline's notice, a readout) leaves it as it was,
    /// so a host writes no checkpoint for it; the views are saved with the next one.
    pub fn durable_hash(&self) -> u64 {
        if let Some(known) = self.durable_memo.get() {
            return known;
        }
        let mut hasher = Fnv::new();
        self.facts.digest_without_stamps().hash(&mut hasher);
        self.pending.digest().hash(&mut hasher);
        self.rulings.digest().hash(&mut hasher);
        self.requests.digest().hash(&mut hasher);
        self.results.digest().hash(&mut hasher);
        self.releases.digest().hash(&mut hasher);
        self.clarifications.as_ref().hash(&mut hasher);
        self.choice_stamps.hash(&mut hasher);
        self.host.hash(&mut hasher);
        let worked_out = hasher.finish();
        self.durable_memo.set(worked_out);
        worked_out
    }

    /// Everything the pipeline holds, in its saved form. A wait is saved as an age at `now_ms`.
    pub fn to_record(&self, now_ms: u64) -> PipelineRecord {
        PipelineRecord {
            schema_version: PIPELINE_RECORD_VERSION,
            pending: self.pending.to_record(now_ms),
            rulings: self.rulings.to_record(),
            requests: self.requests.to_record(),
            results: self.results.to_record(),
            releases: self.releases.to_record(),
            published: self.attachments.to_published(),
            clarifications: self
                .clarifications
                .iter()
                .map(|(seat, options)| (seat.get(), options.clone()))
                .collect(),
            choice_stamps: self
                .choice_stamps
                .iter()
                .map(|(seat, revision)| (seat.get(), *revision))
                .collect(),
            host: self.host.map(SeatId::get),
        }
    }

    /// The table of a restored process: the facts the host loaded and everything the pipeline
    /// held, with the pendings' waits put back against `now_ms`. A record this engine cannot read
    /// is refused with a typed error and nothing is half loaded. The offers are rebuilt from the
    /// pendings and clarifications only, which is right for a pipeline that does not publish the
    /// machine's standing buttons; a pipeline that does loads with `Pipeline::load_table`.
    pub fn from_record(
        table: TableId,
        facts: TableState,
        record: PipelineRecord,
        now_ms: u64,
        limits: &PipelineLimits,
    ) -> Result<Self, PipelineLoadError> {
        Self::from_record_with_standing(table, facts, record, now_ms, limits, None)
    }

    /// `from_record` for a pipeline that publishes the running machine's standing buttons
    /// (`standing` is its registry): the offers are rebuilt with those buttons too, without
    /// counting a new publish, so a restored phone holds the buttons it held when the checkpoint
    /// was taken.
    pub fn from_record_with_standing(
        table: TableId,
        facts: TableState,
        record: PipelineRecord,
        now_ms: u64,
        limits: &PipelineLimits,
        standing: Option<&Registry>,
    ) -> Result<Self, PipelineLoadError> {
        if record.schema_version != PIPELINE_RECORD_VERSION {
            return Err(PipelineLoadError::UnsupportedVersion {
                found: record.schema_version,
                expected: PIPELINE_RECORD_VERSION,
            });
        }
        let pending =
            PendingBook::from_record(record.pending, now_ms, limits.maximum_open_pendings)
                .map_err(PipelineLoadError::Pending)?;
        let rulings =
            RulingBook::from_record(record.rulings).map_err(PipelineLoadError::Rulings)?;
        let requests =
            RequestBook::from_record(record.requests).map_err(PipelineLoadError::Requests)?;
        let results =
            ResultBook::from_record(record.results).map_err(PipelineLoadError::Results)?;
        let releases =
            ReleaseBook::from_record(record.releases).map_err(PipelineLoadError::Releases)?;
        let attachments =
            Attachments::from_published(&record.published).map_err(PipelineLoadError::Published)?;
        let mut loaded = Self {
            table,
            facts,
            pending: Arc::new(pending),
            rulings: Arc::new(rulings),
            requests: Arc::new(requests),
            results: Arc::new(results),
            ends: Arc::new(ResultBook::remembering(limits.maximum_remembered_results)),
            releases: Arc::new(releases),
            attachments: Arc::new(attachments),
            clarifications: Arc::new(
                record
                    .clarifications
                    .into_iter()
                    .map(|(seat, options)| (SeatId::new(seat), options))
                    .collect(),
            ),
            choice_stamps: record
                .choice_stamps
                .into_iter()
                .map(|(seat, revision)| (SeatId::new(seat), revision))
                .collect(),
            host: record.host.map(SeatId::new),
            hash_memo: DigestMemo::empty(),
            durable_memo: DigestMemo::empty(),
        };
        // The offers are derived from the pendings and the clarifications, so they are rebuilt and
        // not saved: a seat that reconnects finds the same buttons by the same pending ids.
        super::offers::restore_offers(&mut loaded, standing).map_err(|error| match error {
            OffersError::Attachments(error) => PipelineLoadError::Offers(error),
            OffersError::Machine(error) => PipelineLoadError::Surface(error),
        })?;
        Ok(loaded)
    }
}

/// The schema version of `PipelineRecord`.
pub const PIPELINE_RECORD_VERSION: u32 = 3;

/// The saved form of everything the pipeline holds beside the table state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineRecord {
    schema_version: u32,
    pending: PendingRecord,
    rulings: RulingRecord,
    requests: RequestRecord,
    results: ResultRecord,
    releases: ReleaseRecord,
    published: PublishedRecord,
    clarifications: Vec<(u64, Vec<ActionId>)>,
    choice_stamps: Vec<(u64, SessionRevision)>,
    host: Option<u64>,
}

/// Why a saved pipeline record was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineLoadError {
    UnsupportedVersion {
        found: u32,
        expected: u32,
    },
    Pending(PendingLoadError),
    Rulings(RulingLoadError),
    Requests(BookLoadError),
    Results(BookLoadError),
    Releases(BookLoadError),
    Published(PublishedLoadError),
    /// The offers derived from the pendings could not be published.
    Offers(crate::projection::AttachmentError),
    /// The running machine could not say what it offers a restored seat.
    Surface(MachineSurfaceError),
}

impl std::fmt::Display for PipelineLoadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion { found, expected } => write!(
                formatter,
                "a pipeline record at version {found} was refused; this engine reads {expected}"
            ),
            Self::Pending(error) => error.fmt(formatter),
            Self::Rulings(error) => error.fmt(formatter),
            Self::Requests(error) | Self::Results(error) | Self::Releases(error) => {
                error.fmt(formatter)
            }
            Self::Published(error) => error.fmt(formatter),
            Self::Offers(error) => error.fmt(formatter),
            Self::Surface(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PipelineLoadError {}
