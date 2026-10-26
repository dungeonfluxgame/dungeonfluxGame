//! The session owner's side of a change of place.
//!
//! The service decides (Jev chooses the exit, the storyteller creates the place) and hands the
//! owner a [`MoveProposal`]. The owner repeats every check against its own current checkpoint, so
//! a stale or forged proposal changes nothing: the party must still stand in the place the move
//! leaves, the exit must exist and be open, and the destination must be exactly the one the exit
//! leads to (a place the run has, or the one new place that is not yet in the journal). It then
//! builds the journal entry as a candidate and commits it through the same owner path as a
//! development jump or an NPC event, publishes the new scene to the screens and writes one
//! `PLACE change` audit line. Nothing is told to the table here; the caller tells the table the
//! returned line, so the table hears about a move only after the move was committed.

use df_model::checkpoint::{CommandInput, GameCommand, GameInput};
use df_observe::OperationContext;
use df_persistence::local_demo_scope::LocalDemoRole;
use df_session::submission::SubmissionOutcome;
use sha2::{Digest, Sha256};

use super::{Actor, record_unavailable, require_readable, session_lost_status, unavailable};
use crate::gameplay::{
    journey, model,
    place::{self, PlaceFailure, PlaceRecord, PlaceRefusal},
};

/// Everything the owner needs to commit one move.
pub(in crate::gameplay) struct MoveProposal {
    pub(in crate::gameplay) operation: df_types::OperationId,
    pub(in crate::gameplay) expected_basis: df_model::checkpoint::Basis,
    /// The place the party leaves; it must be the place it stands in.
    pub(in crate::gameplay) from: String,
    /// The exit of `from` taken, or [`DEV_EXIT`] for a development move to a place the run has.
    pub(in crate::gameplay) exit: String,
    pub(in crate::gameplay) to: String,
    /// The place, when it is new to the run.
    pub(in crate::gameplay) created: Option<PlaceRecord>,
}

impl MoveProposal {
    pub(in crate::gameplay) fn retained_bytes(&self) -> usize {
        self.from.capacity()
            + self.exit.capacity()
            + self.to.capacity()
            + self
                .created
                .as_ref()
                .map_or(0, |_| place::MAXIMUM_PLACE_BYTES)
    }
}

/// A committed move, for the caller to tell the table.
#[derive(Clone, Debug)]
pub(in crate::gameplay) struct Moved {
    pub(in crate::gameplay) revision: df_types::SessionRevision,
    pub(in crate::gameplay) from: String,
    pub(in crate::gameplay) to: String,
    pub(in crate::gameplay) place_name: String,
    pub(in crate::gameplay) exit: String,
    pub(in crate::gameplay) created: bool,
    /// The line the table is told.
    pub(in crate::gameplay) line: String,
    /// What the screens show on arrival.
    pub(in crate::gameplay) narration: String,
}

pub(in crate::gameplay) enum MoveOutcome {
    Moved(Box<Moved>),
    Refused(PlaceRefusal),
}

fn refused(proposal: &MoveProposal, reason: PlaceRefusal) -> Result<MoveOutcome, tonic::Status> {
    place::audit_refused(&proposal.from, &proposal.to, &proposal.exit, reason.label());
    Ok(MoveOutcome::Refused(reason))
}

/// A refused journey is still an event at the table. Describe only the known restriction;
/// generation failure must not invent a collapsed road or another persistent world fact.
pub(in crate::gameplay) fn publish_travel_refusal(
    log: &crate::gameplay::table_log::TableLog,
    reason: PlaceRefusal,
) {
    let line = travel_refusal_line(reason);
    log.push(
        "Narrator",
        df_protocol::common::TableRole::Narrator,
        0,
        df_protocol::common::TableKind::Action,
        line,
    );
}

pub(in crate::gameplay) fn travel_refusal_line(reason: PlaceRefusal) -> &'static str {
    match reason {
        PlaceRefusal::TravelBlocked(reason) => reason,
        PlaceRefusal::Closed => "The way is closed. You remain where you are.",
        PlaceRefusal::StoryPlace => {
            "You have heard of that destination, but have not found a route there from here."
        }
        PlaceRefusal::NoExit => "You have not found a way out in that direction.",
        PlaceRefusal::NotHere => "Your party has already moved on from that departure point.",
        PlaceRefusal::AlreadyKnown => "You already know that place; take the route back to it.",
        PlaceRefusal::CreationUnavailable | PlaceRefusal::InvalidPlace | PlaceRefusal::TooLarge => {
            "You look for a route ahead, but cannot make it out yet. Your party stays together here."
        }
        PlaceRefusal::Duplicate => "Your party has already made that journey.",
        PlaceRefusal::RunFull | PlaceRefusal::Refused => {
            "Your party cannot set off that way yet. You remain here."
        }
    }
}

impl Actor {
    /// Commits one change of place, or says why not. `Err` is only an authority failure.
    pub(super) fn place_move(
        &mut self,
        credential: &[u8; 32],
        proposal: MoveProposal,
    ) -> Result<MoveOutcome, tonic::Status> {
        if self.session_lost() {
            return Err(session_lost_status());
        }
        require_readable(self.fenced)?;
        if self.issuer.authenticate(credential).map_err(unavailable)? != LocalDemoRole::Player {
            return Err(tonic::Status::permission_denied(
                "a change of place needs a seated player binding",
            ));
        }
        let member = self.dialogue_member(credential)?;
        let current = self.owner.checkpoint().clone();
        if proposal.expected_basis != current.basis() {
            return Err(tonic::Status::aborted("the departure's scene changed"));
        }
        let hero_entity = journey::player_entity(member, &current).map_err(unavailable)?;
        let hero = journey::name(current.state(), hero_entity).map_err(unavailable)?;
        let planned = match place::plan(
            &current,
            &hero,
            proposal.operation,
            &proposal.from,
            &proposal.exit,
            &proposal.to,
            proposal.created.as_ref(),
        ) {
            Ok(planned) => planned,
            Err(PlaceFailure::Refused(reason)) => return refused(&proposal, reason),
            Err(PlaceFailure::State(error)) => return Err(unavailable(error)),
        };
        let candidate = planned.candidate;
        let (entry, to_name, arrival) = (planned.entry, planned.to_name, planned.arrival);
        let marker = format!(
            "place-move:{}:{}:{}:{}",
            journey::hex(proposal.operation.as_bytes()),
            proposal.from,
            proposal.exit,
            proposal.to
        );
        let speaker = current
            .state()
            .characters
            .iter()
            .find(|character| character.owner == member)
            .map(|character| character.entity)
            .ok_or_else(|| tonic::Status::failed_precondition("seat has no hero"))?;
        let input = GameInput::Game(CommandInput {
            basis: current.basis(),
            observed_revision: current.basis().revision,
            operation: proposal.operation,
            member,
            command: GameCommand::Speak {
                speaker,
                text: marker.clone(),
                conversation: None,
            },
        });
        let fingerprint: [u8; 32] = Sha256::digest(marker.as_bytes()).into();
        let scope = self
            .issuer
            .issue(
                credential,
                model::random().map_err(unavailable)?,
                fingerprint,
                input,
                model::pins().map_err(unavailable)?,
            )
            .map_err(unavailable)?;
        let context = OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        match self
            .owner
            .commit_owner_candidate(&scope, &candidate, &context)
        {
            SubmissionOutcome::Confirmed(receipt) => {
                if !self.owner.is_current() {
                    self.fence();
                }
                if let Err(error) = journey::living_scene::publish(self.owner.checkpoint()) {
                    record_unavailable("place-publication", &format!("{error:?}"));
                }
                self.refresh_dev_moment();
                let revision = receipt.basis().revision;
                let created = proposal.created.is_some();
                place::audit_changed(&proposal.from, &proposal.to, &proposal.exit, created);
                // The story memory hears of the move, so later places and lines agree with it.
                let position =
                    format!("place-move:{}", journey::hex(proposal.operation.as_bytes()));
                crate::gameplay::memory::capture::scene_change(
                    crate::gameplay::memory::capture::SceneChange {
                        position: &position,
                        revision: i64::try_from(revision.sequence()).ok(),
                        place: &proposal.from,
                        hero: &hero,
                        entity_id: &proposal.exit,
                        description: &entry.line,
                        tag: "place_change",
                        involved: std::slice::from_ref(&proposal.to),
                    },
                );
                Ok(MoveOutcome::Moved(Box::new(Moved {
                    revision,
                    from: proposal.from,
                    to: proposal.to,
                    place_name: to_name,
                    exit: proposal.exit,
                    created,
                    line: entry.line,
                    narration: arrival,
                })))
            }
            SubmissionOutcome::OperationConflict => Err(tonic::Status::aborted(
                "change of place conflicts with another operation",
            )),
            SubmissionOutcome::Refused(error) => {
                place::audit_refused(
                    &proposal.from,
                    &proposal.to,
                    &proposal.exit,
                    &format!("owner_refused:{error:?}"),
                );
                Err(unavailable(error))
            }
            SubmissionOutcome::LookupRequired | SubmissionOutcome::ExpiredOrIndeterminate => {
                self.fence();
                Err(tonic::Status::unavailable(
                    "change of place outcome unavailable",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_actor_travel_refusals_are_visible_and_do_not_expose_generation_errors() {
        let log = crate::gameplay::table_log::TableLog::new();
        for reason in [
            PlaceRefusal::Closed,
            PlaceRefusal::CreationUnavailable,
            PlaceRefusal::TravelBlocked("Dain bars the courier's way."),
        ] {
            publish_travel_refusal(&log, reason);
        }
        let lines = log.lines();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].text.contains("closed"));
        assert!(lines[1].text.contains("stays together here"));
        assert!(!lines[1].text.contains("generation"));
        assert!(lines[2].text.contains("Dain"));
    }
}
