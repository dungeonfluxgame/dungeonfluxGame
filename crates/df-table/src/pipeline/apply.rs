//! Stage 4, APPLY: one in-memory commit of everything a turn decided (C, C.0, C.4).
//!
//! It owns `Staged`, the changes the earlier stages decided but did not make, and `commit`, which
//! makes them together or not at all: the table effects (the rules owner's and the machine's, then
//! the new saved state of the running machine as an `ActiveSlotUpdated` effect), the pending
//! resolutions opened, replaced or closed and the marks on them, the rulings stored or revoked and
//! the resource requests asked for. The effects are applied first and all or nothing
//! (`PipelineTable::apply_effects`) and the pending changes are checked before anything is
//! applied, so a refusal leaves everything as it was.
//!
//! Nothing here is visible outside the table before the host's durable commit (O.16 S1, group
//! commit). The events about changes (`EffectApplied`, a pending opened, a ruling stored,
//! `TurnApplied`) are not recorded: they go to `Turn::on_commit` and the host releases them with
//! the turn's result after its commit. A resource request is recorded as an intent in the
//! `RequestBook` and issued to the broker by `Pipeline::issue_requests`, also after the commit. A
//! refused batch is undone by restoring the checkpoint the host took before it.
//!
//! It never decides anything and never calls a machine, the rules owner or a port. A refused
//! effect is recorded once as `EffectRefused` and the input is declined `not_viable`; a full
//! pending book declines `table_busy`. A commit whose operation id the table state has applied
//! before cannot be reached: the result book finds a duplicate before any port is asked.

use crate::effects::{ApplyOutcome, TableEffect};
use crate::envelope::PendingId;
use crate::event::{PipelineStep, TableEvent};
use crate::identity::{SceneId, SeatId};
use crate::machine::{MachineId, RequestKey};
use crate::machine_slot::MachineSlot;
use crate::outcome::{Decline, DeclineCode, OutcomeKind};
use crate::pending::{PendingError, PendingMark, PendingResolution, ScopedRuling};
use crate::ports::{PurposeName, RulingKey, RulingScope};
use crate::table_state::ResultStamp;

use super::table_access::{PipelineTable, Running};
use super::turn::Turn;

/// A change to the stored rulings.
#[derive(Debug, Clone)]
pub(super) enum RulingChange {
    Store(ScopedRuling),
    Reconfirm {
        key: RulingKey,
        scope: RulingScope,
        scene: Option<SceneId>,
    },
    Revoke {
        key: RulingKey,
        scope: RulingScope,
        scene: Option<SceneId>,
    },
}

/// What happens to the seat's open clarification when the turn commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClarificationChange {
    Keep,
    Clear(SeatId),
}

/// A resource request a machine's step asked for, to be recorded in the commit.
#[derive(Debug, Clone)]
pub(super) struct StagedRequest {
    pub(super) machine: MachineId,
    pub(super) key: RequestKey,
    pub(super) purpose: PurposeName,
}

/// The changes of one turn, decided and not yet made.
#[derive(Debug)]
pub(super) struct Staged {
    pub(super) effects: Vec<TableEffect>,
    /// The new saved state of the machine that runs, when a step changed it.
    pub(super) slot: Option<MachineSlot>,
    pub(super) open: Vec<PendingResolution>,
    pub(super) replace: Vec<PendingResolution>,
    pub(super) close: Vec<PendingId>,
    pub(super) marks: Vec<PendingMark>,
    pub(super) rulings: Vec<RulingChange>,
    pub(super) requests: Vec<StagedRequest>,
    /// Reservations the closed pendings give up, told to the rules owner after the commit.
    pub(super) releases: Vec<StagedRelease>,
    /// The broker handles of requests whose result this turn consumed or refused.
    pub(super) finished_requests: Vec<u64>,
    pub(super) clarification: ClarificationChange,
}

/// A reservation to release once the commit that cancelled its pending is durable.
#[derive(Debug, Clone)]
pub(super) struct StagedRelease {
    pub(super) pending: PendingId,
    pub(super) seat: SeatId,
    pub(super) action: crate::ports::ActionId,
}

impl Staged {
    pub(super) fn new() -> Self {
        Self {
            effects: Vec::new(),
            slot: None,
            open: Vec::new(),
            replace: Vec::new(),
            close: Vec::new(),
            marks: Vec::new(),
            rulings: Vec::new(),
            requests: Vec::new(),
            releases: Vec::new(),
            finished_requests: Vec::new(),
            clarification: ClarificationChange::Keep,
        }
    }
}

/// The stamp a request is made under: the focus scene, its generation and the table's revision.
fn stamp_of(table: &PipelineTable) -> Option<ResultStamp> {
    let snapshot = table.facts().snapshot();
    let scene = snapshot.focus_scene()?;
    Some(ResultStamp {
        scene,
        generation: snapshot.scene(scene)?.generation(),
        revision: snapshot.revision(),
    })
}

/// Makes the staged changes together or not at all. On refusal nothing has changed, the events of
/// the changes that did not happen are dropped, and the refusal has been recorded.
pub(super) fn commit(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    staged: Staged,
) -> Result<(), Decline> {
    let result = commit_staged(turn, table, staged);
    if result.is_err() {
        turn.on_commit.clear();
    }
    result
}

fn commit_staged(
    turn: &mut Turn<'_, '_>,
    table: &mut PipelineTable,
    staged: Staged,
) -> Result<(), Decline> {
    let started = turn.now();
    if let Err(error) = table
        .pending()
        .can_apply(&staged.close, &staged.replace, &staged.open)
    {
        let code = match error {
            PendingError::TooManyOpen => DeclineCode::TableBusy,
            PendingError::IdNotNext
            | PendingError::OperationAlreadyOpened
            | PendingError::Unknown => DeclineCode::NotViable,
        };
        turn.stage_done(PipelineStep::Apply, started, Some(OutcomeKind::Declined));
        return Err(Decline::with_default_reason(code));
    }
    let scope = table.scope();
    let stamp = stamp_of(table);
    let mut effects = staged.effects;
    if let Some(slot) = staged.slot {
        let snapshot = table.facts().snapshot();
        if let Some(Running { home, .. }) = table.running(&snapshot) {
            effects.push(TableEffect::ActiveSlotUpdated { home, slot });
        }
    }
    match table.apply_effects(&scope, turn.correlation, &effects) {
        Ok(ApplyOutcome::Applied(events)) => turn.on_commit.extend(events),
        Ok(ApplyOutcome::AlreadyApplied) => {
            turn.stage_done(PipelineStep::Apply, started, Some(OutcomeKind::Declined));
            return Err(Decline::with_default_reason(DeclineCode::NotViable));
        }
        Err(error) => {
            turn.emit(&error.event(&scope, turn.correlation));
            turn.stage_done(PipelineStep::Apply, started, Some(OutcomeKind::Declined));
            return Err(Decline::with_default_reason(DeclineCode::NotViable));
        }
    }
    install(
        table,
        staged.open,
        staged.replace,
        &staged.close,
        &staged.marks,
    );
    for change in staged.rulings {
        match change {
            RulingChange::Store(ruling) => table.rulings_mut().store(ruling),
            RulingChange::Reconfirm { key, scope, scene } => {
                table.rulings_mut().reconfirm(&key, scope, scene);
            }
            RulingChange::Revoke { key, scope, scene } => {
                table.rulings_mut().revoke(&key, scope, scene);
            }
        }
    }
    for handle in staged.finished_requests {
        table.requests_mut().take_by_handle(handle);
    }
    for request in staged.requests {
        table.requests_mut().record_intent(
            request.machine,
            request.key,
            request.purpose,
            stamp,
            turn.correlation,
        );
    }
    for release in staged.releases {
        table.releases_mut().record(
            release.pending.get(),
            release.seat,
            release.action,
            turn.correlation,
        );
    }
    if let ClarificationChange::Clear(seat) = staged.clarification {
        table.clear_clarification(seat);
    }
    table.forget_absent_seats();
    turn.effects = effects;
    turn.committed = true;
    turn.on_commit.push(TableEvent::TurnApplied {
        scope: table.scope(),
        correlation: turn.correlation,
        ms: turn.elapsed_since(turn.started_ms),
    });
    turn.stage_done(PipelineStep::Apply, started, None);
    Ok(())
}

/// Makes the pending changes the book already checked.
fn install(
    table: &mut PipelineTable,
    open: Vec<PendingResolution>,
    replace: Vec<PendingResolution>,
    close: &[PendingId],
    marks: &[PendingMark],
) {
    if open.is_empty() && replace.is_empty() && close.is_empty() && marks.is_empty() {
        return;
    }
    let book = table.pending_mut();
    book.apply_changes(close, replace, open);
    for mark in marks {
        book.mark(*mark);
    }
}
