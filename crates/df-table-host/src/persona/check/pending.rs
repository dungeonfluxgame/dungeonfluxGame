//! A seated hero rolls their own dice: when a check is called for a character a seat plays, the
//! call is said, and then the table waits for the player to roll (`PendingRoll`, which the seat's
//! view shows) before the host draws the dice. The seat's tap answers it
//! (`answer_pending_roll`). There is no deadline: the host never rolls in a player's place. The
//! roll is withdrawn only when the run ends (its future dropped) or the seat no longer plays the
//! hero, which the wait looks at every `ROLL_WATCH_MS`. One roll waits at a time: check runs take
//! the table's turn one after another.
//!
//! Audit lines: `CHECK awaiting_roll seat= roll_id=`, `CHECK roll_waited seat= roll_id= ms=` each
//! watch while it waits, then `CHECK roll_tapped seat= waited_ms=` or `CHECK roll_withdrawn seat=
//! roll_id= reason=seat_left|replaced waited_ms=`.

use std::fmt;
use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use df_playground::fixture::{RollMode as ViewMode, RollTarget};
use df_playground::seat::wire::PendingRoll;
use df_rules::roll::{Ability, RollRequest, Skill};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::numbers::{target_word, test_name};
use super::request::CheckCall;
use super::view::mode_of;
use crate::state::AppState;

/// Why a seat's tap does not answer a roll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum PendingRollError {
    /// No roll waits for the seat.
    NoPendingRoll { seat: u64 },
    /// The roll the tap names is not the one waiting (one already rolled, or the wait ran out).
    StaleRoll { roll_id: String },
}

impl PendingRollError {
    pub fn key(&self) -> &'static str {
        match self {
            Self::NoPendingRoll { .. } => "no_pending_roll",
            Self::StaleRoll { .. } => "stale_roll",
        }
    }
}

impl fmt::Display for PendingRollError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPendingRoll { seat } => write!(formatter, "no roll waits for seat {seat}"),
            Self::StaleRoll { roll_id } => {
                write!(formatter, "the roll {roll_id} is no longer waiting")
            }
        }
    }
}

impl std::error::Error for PendingRollError {}

/// How a wait for the player's roll ended: never by running out of time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RollWait {
    /// The player tapped: the dice are drawn.
    Tapped,
    /// The seat no longer plays the hero, or the roll was taken off the seat: nobody rolls.
    Withdrawn,
}

/// Takes a waiting roll off its seat when the wait ends, however it ends (a dropped run included),
/// unless another roll has taken the slot since.
struct Withdraw<'a> {
    rolls: &'a PendingRolls,
    roll_id: String,
}

impl Drop for Withdraw<'_> {
    fn drop(&mut self) {
        let mut slot = self.rolls.slot();
        if slot
            .as_ref()
            .is_some_and(|waiting| waiting.roll.roll_id == self.roll_id)
        {
            *slot = None;
        }
    }
}

/// The roll waiting for a seat, and the way to wake the check run that waits for it.
struct Waiting {
    seat: u64,
    roll: PendingRoll,
    tapped: oneshot::Sender<()>,
}

/// The roll a seat is asked to make now; at most one, as one check runs at a time.
#[derive(Default)]
pub(super) struct PendingRolls {
    waiting: Mutex<Option<Waiting>>,
}

impl PendingRolls {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Waiting>> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The roll waiting for `seat`, if any.
    pub(super) fn of(&self, seat: u64) -> Option<PendingRoll> {
        self.slot()
            .as_ref()
            .filter(|waiting| waiting.seat == seat)
            .map(|waiting| waiting.roll.clone())
    }

    /// The seat's tap on `roll_id`: the check run waiting for it rolls.
    pub(super) fn answer(&self, seat: u64, roll_id: &str) -> Result<(), PendingRollError> {
        let mut slot = self.slot();
        match slot.as_ref() {
            Some(waiting) if waiting.seat == seat && waiting.roll.roll_id == roll_id => {}
            Some(waiting) if waiting.seat == seat => {
                return Err(PendingRollError::StaleRoll {
                    roll_id: roll_id.to_owned(),
                });
            }
            _ => return Err(PendingRollError::NoPendingRoll { seat }),
        }
        if let Some(waiting) = slot.take() {
            // The run may have just stopped waiting; then the tap simply finds it gone.
            let _ = waiting.tapped.send(());
        }
        Ok(())
    }

    /// Publishes `roll` for `seat` and waits for its tap with no deadline: every `watch` it logs
    /// that it still waits and asks `still_seated` whether the seat still plays the hero, and
    /// withdraws the roll when it does not. The roll is taken off the seat however the wait ends.
    pub(super) async fn wait<Seated, Asked>(
        &self,
        app: &AppState,
        (seat, roll): (u64, PendingRoll),
        watch: Duration,
        still_seated: Seated,
    ) -> RollWait
    where
        Seated: Fn() -> Asked,
        Asked: Future<Output = bool>,
    {
        let (tapped, mut tap) = oneshot::channel();
        let roll_id = roll.roll_id.clone();
        *self.slot() = Some(Waiting { seat, roll, tapped });
        let _withdraw = Withdraw {
            rolls: self,
            roll_id: roll_id.clone(),
        };
        app.record_line(format!("CHECK awaiting_roll seat={seat} roll_id={roll_id}"));
        let started = Instant::now();
        let waited_ms = || u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let withdrawn = |reason: &str| {
            app.record_line(format!(
                "CHECK roll_withdrawn seat={seat} roll_id={roll_id} reason={reason} waited_ms={}",
                waited_ms()
            ));
            RollWait::Withdrawn
        };
        loop {
            match tokio::time::timeout(watch, &mut tap).await {
                Ok(Ok(())) => {
                    app.record_line(format!(
                        "CHECK roll_tapped seat={seat} waited_ms={}",
                        waited_ms()
                    ));
                    return RollWait::Tapped;
                }
                // The slot was taken by another roll: this one no longer waits.
                Ok(Err(_)) => return withdrawn("replaced"),
                Err(_) => {
                    app.record_line(format!(
                        "CHECK roll_waited seat={seat} roll_id={roll_id} ms={}",
                        waited_ms()
                    ));
                    if !still_seated().await {
                        return withdrawn("seat_left");
                    }
                }
            }
        }
    }
}

/// The roll `call` asks for as the seat is shown it, named `roll_id`.
pub(super) fn pending_of(roll_id: String, call: &CheckCall, request: &RollRequest) -> PendingRoll {
    let (mode, target) = match request {
        RollRequest::D20(d20) => (
            mode_of(d20.mode),
            d20.target.map(|target| RollTarget {
                label: target_word(target).to_owned(),
                value: i32::from(target.value),
            }),
        ),
        RollRequest::PassivePerception(_) | RollRequest::Dice(_) => (ViewMode::Normal, None),
    };
    PendingRoll {
        roll_id,
        // A tool check is named by its tool, as the table's card names it.
        title: super::view::tool_title(call).unwrap_or_else(|| test_name(request)),
        kind: call.kind.key().to_owned(),
        ability: call.ability.map(Ability::key).map(str::to_owned),
        skill: call.skill.map(Skill::key).map(str::to_owned),
        mode,
        target,
    }
}

/// The roll the dungeon master is waiting for `seat` to make, if any (the seat's view shows it).
pub(crate) fn pending_roll(app: &AppState, seat: u64) -> Option<PendingRoll> {
    app.check_rolls()?.pending.of(seat)
}

/// The seat's tap on its waiting roll `roll_id`: the check goes on, and the host rolls the dice.
pub(crate) fn answer_pending_roll(
    app: &AppState,
    seat: u64,
    roll_id: &str,
) -> Result<(), PendingRollError> {
    match app.check_rolls() {
        Some(checks) => checks.pending.answer(seat, roll_id),
        None => Err(PendingRollError::NoPendingRoll { seat }),
    }
}
