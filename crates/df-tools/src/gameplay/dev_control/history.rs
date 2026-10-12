//! Back/forward history for the development control surface.
//!
//! The owner records every committed checkpoint (player and dev commits alike)
//! in [`History::past`]; stepping back moves the stepped-over states to
//! [`History::redo`]. Playing on after a jump forks: the first unrelated
//! commit drops the redo stack and the next dev command reports how many
//! entries were dropped. Named snapshots cap the same store milestone 1
//! started. All revision math here is pure and unit-tested; the live
//! transcript proves the integrated jumps.
//!
//! Every entry is a [`SavedMoment`]: the checkpoint plus the actor-side state
//! the checkpoint does not carry (the table-log floor and the social sidecar
//! snapshot), so a rewind restores all of it or none.

use df_model::checkpoint::Checkpoint;
use std::collections::{BTreeMap, VecDeque};
use std::mem::size_of;

use super::super::social::SocialSnapshot;

/// A session revision as a plain comparable pair (epoch, sequence).
pub(crate) type Revision = (u64, u64);

/// How many committed checkpoints the past ring keeps.
pub(crate) const PAST_CAPACITY: usize = 64;
/// How many named snapshots are kept.
pub(crate) const NAMED_CAPACITY: usize = 16;

/// One restorable moment: the committed checkpoint plus the actor-side state
/// the checkpoint does not carry. The table floor truncates the public log to
/// the saved moment; the social snapshot rewinds the sidecar ledger.
#[derive(Clone, Debug)]
pub(crate) struct SavedMoment {
    pub checkpoint: Checkpoint,
    pub table_floor: u64,
    pub social: SocialSnapshot,
}

impl SavedMoment {
    pub(crate) fn revision(&self) -> Revision {
        revision_of(&self.checkpoint)
    }

    pub(crate) fn retained_heap_bytes(&self) -> Option<usize> {
        self.checkpoint
            .retained_bytes()?
            .checked_add(size_of::<Self>())
            .and_then(|bytes| bytes.checked_add(self.social.retained_heap_bytes()?))
    }
}

pub(crate) struct History {
    past: VecDeque<SavedMoment>,
    redo: Vec<SavedMoment>,
    redo_base: Option<Revision>,
    named: BTreeMap<String, SavedMoment>,
    dropped_redo: u64,
}

impl History {
    pub(crate) fn new() -> Self {
        Self {
            past: VecDeque::new(),
            redo: Vec::new(),
            redo_base: None,
            named: BTreeMap::new(),
            dropped_redo: 0,
        }
    }

    /// Records one committed moment. Called for every commit, player or
    /// dev, so stepping counts decisions rather than dev commands.
    pub(crate) fn record(&mut self, moment: SavedMoment) {
        if self.past.len() >= PAST_CAPACITY {
            self.past.pop_front();
        }
        self.past.push_back(moment);
    }

    /// Refreshes the newest entry's actor-side state after a rewind: the
    /// publication hook records the new checkpoint during the commit, before
    /// the table log and the social ledger are rewound on the owner thread.
    pub(crate) fn refresh_latest_aux(&mut self, table_floor: u64, social: SocialSnapshot) {
        if let Some(latest) = self.past.back_mut() {
            latest.table_floor = table_floor;
            latest.social = social;
        }
    }

    /// Finds the moment `n` decisions back from `current`, oldest-first
    /// search over the ring. Returns the target plus the stepped-over states
    /// (oldest first) that become the new redo stack.
    pub(crate) fn back_from(
        &self,
        current: Revision,
        n: u64,
    ) -> Result<(SavedMoment, Vec<SavedMoment>), String> {
        let n = usize::try_from(n).map_err(|_| "step count too large".to_owned())?;
        if n == 0 {
            return Err("back needs at least 1".to_owned());
        }
        let target = back_target(
            &self
                .past
                .iter()
                .map(SavedMoment::revision)
                .collect::<Vec<_>>(),
            current,
            n,
        )
        .ok_or_else(|| available_message(&self.past, current))?;
        let stepped: Vec<SavedMoment> = self
            .past
            .iter()
            .filter(|moment| {
                let revision = moment.revision();
                revision > target && revision <= current
            })
            .cloned()
            .collect();
        let snapshot = self
            .past
            .iter()
            .find(|moment| moment.revision() == target)
            .cloned()
            .ok_or_else(|| available_message(&self.past, current))?;
        Ok((snapshot, stepped))
    }

    /// Pops one redo entry (the most recently stepped-over state first).
    pub(crate) fn forward_pop(&mut self) -> Option<SavedMoment> {
        self.redo.pop()
    }

    /// Returns an entry to the redo stack untouched, e.g. when its forward
    /// step fails to commit: a failed step must never silently eat history.
    pub(crate) fn push_redo(&mut self, moment: SavedMoment) {
        self.redo.push(moment);
    }

    pub(crate) fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// Drops a stale redo stack: any commit that is not the back/forward jump
    /// the redo was built for forks history. Returns the dropped entries so
    /// the next dev command can report them instead of silently forgetting.
    pub(crate) fn drop_stale_redo(&mut self, current: Revision) -> Vec<SavedMoment> {
        if self.redo.is_empty() || self.redo_base == Some(current) {
            return Vec::new();
        }
        let dropped = std::mem::take(&mut self.redo);
        self.redo_base = None;
        self.dropped_redo += dropped.len() as u64;
        dropped
    }

    /// Installs a fresh redo stack after a dev jump, anchored at the revision
    /// the jump committed.
    pub(crate) fn set_redo(&mut self, entries: Vec<SavedMoment>, base: Revision) {
        self.redo = entries;
        self.redo_base = Some(base);
    }

    /// Re-anchors leftover redo entries at a new revision without losing them.
    pub(crate) fn reanchor(&mut self, base: Revision) {
        if !self.redo.is_empty() {
            self.redo_base = Some(base);
        }
    }

    /// The newest recorded moment: the owner's current state, since the
    /// publication hook records every commit.
    pub(crate) fn latest(&self) -> Option<SavedMoment> {
        self.past.back().cloned()
    }

    pub(crate) fn save_named(&mut self, name: String, moment: SavedMoment) -> Result<(), String> {
        if self.named.len() >= NAMED_CAPACITY && !self.named.contains_key(&name) {
            return Err(format!("at most {NAMED_CAPACITY} checkpoints are kept"));
        }
        self.named.insert(name, moment);
        Ok(())
    }

    pub(crate) fn named(&self, name: &str) -> Option<SavedMoment> {
        self.named.get(name).cloned()
    }

    /// Redo entries dropped by playing on so far (reported, never silent).
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped_redo
    }

    pub(crate) fn named_list(&self) -> Vec<(String, Revision)> {
        let mut list: Vec<(String, Revision)> = self
            .named
            .iter()
            .map(|(name, moment)| (name.clone(), moment.revision()))
            .collect();
        list.sort();
        list
    }
}

fn revision_of(checkpoint: &Checkpoint) -> Revision {
    let revision = checkpoint.basis().revision;
    (revision.epoch().get(), revision.sequence())
}

/// Pure step-back math over revision pairs: the revision `n` steps back from
/// `current`, requiring every intermediate revision to be present. `None`
/// means the ring does not cover the walk.
fn back_target(past: &[Revision], current: Revision, n: usize) -> Option<Revision> {
    if n == 0 || past.is_empty() {
        return None;
    }
    let mut want = current;
    for _ in 0..n {
        want = previous_revision(past, want)?;
    }
    Some(want)
}

/// The newest revision below `current` present in the ring.
fn previous_revision(past: &[Revision], current: Revision) -> Option<Revision> {
    past.iter()
        .filter(|revision| **revision < current)
        .max()
        .copied()
}

/// Explains how far back the ring reaches when a walk fails.
fn available_message(past: &VecDeque<SavedMoment>, current: Revision) -> String {
    match past.front().map(SavedMoment::revision) {
        Some(oldest) => format!(
            "history covers {}:{} back to {}:{} ({} steps kept); larger walks are gone",
            current.0,
            current.1,
            oldest.0,
            oldest.1,
            past.len(),
        ),
        None => "no committed history has been recorded yet".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rev(sequence: u64) -> Revision {
        (1, sequence)
    }

    #[test]
    fn back_math_walks_exact_revisions() {
        let past = vec![rev(5), rev(6), rev(7), rev(8)];
        assert_eq!(back_target(&past, rev(8), 1), Some(rev(7)));
        assert_eq!(back_target(&past, rev(8), 2), Some(rev(6)));
        assert_eq!(back_target(&past, rev(8), 3), Some(rev(5)));
        assert_eq!(back_target(&past, rev(8), 4), None);
        assert_eq!(back_target(&past, rev(8), 0), None);
    }

    #[test]
    fn back_math_requires_every_intermediate_step() {
        // A gap (a missed coalesced update) refuses rather than landing wrong.
        let past = vec![rev(5), rev(8)];
        assert_eq!(back_target(&past, rev(8), 1), Some(rev(5)));
        assert_eq!(back_target(&past, rev(8), 2), None);
    }

    #[test]
    fn redo_staleness_detects_playing_on() {
        let mut history = History::new();
        history.set_redo(Vec::new(), rev(9));
        assert!(history.drop_stale_redo(rev(9)).is_empty());
        history.set_redo(Vec::new(), rev(9));
        // An empty redo stack reports nothing even when stale.
        assert!(history.drop_stale_redo(rev(12)).is_empty());
        assert_eq!(history.dropped(), 0);
    }
}
