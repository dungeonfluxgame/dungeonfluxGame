//! Stage 0, ADMIT: who sent it, whether it may run, and what kind of input it is (C, C.0, H.2).
//!
//! The seat comes from the envelope, which the host built from the authenticated session, and
//! never from the message. Duplicates, the per-seat bound and the rate limit were settled by the
//! owner queue before the input was served, so this stage adds only what the queue cannot know:
//! a seat that is not at the table may send only an action the active machine declares for an
//! unseated seat (the lobby's `join`); a private message is not run yet; a view older than the
//! seat's last change of legal choices is checked again, never dropped, which is recorded as
//! `InputRechecked` because CHECK always asks the rules owner afresh; and a tap that answers an
//! open clarification is read as clarified, so the harm check of C.5 applies to it.
//!
//! The table's own judgement comes first: a paused table declines every play input `time_locked`,
//! and an idle one runs it after resuming (D.3). It never reads the text of a line and never calls
//! a port.

use crate::envelope::{InputEnvelope, PlayerInput};
use crate::identity::SeatId;
use crate::outcome::{Decline, DeclineCode};
use crate::ports::ActionId;
use crate::private_data::PlayerText;

use crate::table_state::PlayInput;

use super::events::PipelineEvent;
use super::table_access::PipelineTable;
use super::turn::Turn;

/// How the action of an input came to be: the harm check of C.5 skips a direct tap on an offered
/// choice and applies to every other reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reading {
    /// A tap on a choice the phone offered.
    Offered,
    /// A tap that answered a clarification.
    Clarified,
    /// A line of free text, interpreted.
    Interpreted,
}

impl Reading {
    pub(super) fn needs_harm_check(self) -> bool {
        match self {
            Self::Offered => false,
            Self::Clarified | Self::Interpreted => true,
        }
    }
}

/// What ADMIT decided about a player input.
pub(super) enum Admission<'i> {
    Declined(Decline),
    /// The safety card: no model call, no check, the host pauses (F.7).
    Pause,
    Answer {
        pending: crate::envelope::PendingId,
        choice: &'i ActionId,
    },
    Tap {
        action: &'i ActionId,
        reading: Reading,
    },
    Text(&'i PlayerText),
}

pub(super) fn admit<'i>(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    envelope: &InputEnvelope,
    seat: SeatId,
    input: &'i PlayerInput,
) -> Admission<'i> {
    if table.has_seat(seat) && table.view_is_stale(seat, envelope.view().revision()) {
        turn.stage_outcome(turn.pipeline_event(|scope, correlation| {
            PipelineEvent::InputRechecked { scope, correlation }
        }));
    }
    if !matches!(
        input,
        PlayerInput::SafetyCard | PlayerInput::PrivateMessage { .. }
    ) {
        match table.facts().snapshot().judge_play_input() {
            Ok(PlayInput::Proceed) => {}
            Ok(PlayInput::WakeIdleTable) => turn.wake_idle = true,
            Err(decline) => return Admission::Declined(decline),
        }
    }
    match input {
        PlayerInput::SafetyCard => Admission::Pause,
        PlayerInput::PendingAnswer { pending, choice } => Admission::Answer {
            pending: *pending,
            choice,
        },
        PlayerInput::PrivateMessage { .. } => {
            Admission::Declined(Decline::with_default_reason(DeclineCode::NotSupportedYet))
        }
        PlayerInput::Tap { action } => {
            if !table.has_seat(seat) && !declared_for_unseated(turn, table, action) {
                return Admission::Declined(Decline::with_default_reason(DeclineCode::NotViable));
            }
            let clarified = table
                .clarification_of(seat)
                .is_some_and(|options| options.contains(action));
            Admission::Tap {
                action,
                reading: if clarified {
                    Reading::Clarified
                } else {
                    Reading::Offered
                },
            }
        }
        PlayerInput::SubmitText { text, .. } | PlayerInput::Say { text, .. } => {
            if table.has_seat(seat) {
                Admission::Text(text)
            } else {
                Admission::Declined(Decline::with_default_reason(DeclineCode::NotViable))
            }
        }
    }
}

/// Whether the active machine lets a seat that is not at the table send this action.
pub(super) fn declared_for_unseated(
    turn: &Turn<'_, '_>,
    table: &PipelineTable,
    action: &ActionId,
) -> bool {
    table
        .running_machine()
        .and_then(|id| turn.pipeline.registry.machine(&id))
        .is_some_and(|machine| machine.spec().unseated_actions().contains(action))
}

/// The answer to a seat that asks to split off from the party: the table's own judgement, then
/// `not_supported_yet` for a split the table would allow, because moving one seat to another scene
/// is not built (RES25, C6).
pub(super) fn split_decline(table: &PipelineTable, seat: SeatId) -> Decline {
    match table.facts().snapshot().judge_split(seat) {
        Err(decline) => decline,
        Ok(()) => Decline::with_default_reason(DeclineCode::NotSupportedYet),
    }
}
