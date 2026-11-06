//! Submitting inputs: building the envelope, admitting it to the queue, collecting the submits
//! that wait in the mailbox into one batch, and the engine's own inputs (a machine's hand-off
//! proposal and the results of resource requests).
//!
//! The caller names a seat and an input; the owner makes the `InputEnvelope`, stamps it with the
//! revision it was drawn from, and lets the real `OwnerQueue` decide whether it may wait. A
//! refusal is a typed report, not an error of the call: a duplicate, a rate limit and a full lane
//! are what the queue answered, and `queue show` lists them. While the owner backs off from a
//! failed commit it refuses new input `table_busy` without admitting it. A repeat of an operation
//! committed in an earlier run is not the queue's to know: the pipeline answers it from the results
//! it keeps in the table and saves in every checkpoint.
//!
//! `serve_submits` is where the group commit of S1 is formed in the live path: every submit already
//! in the mailbox joins the batch, within the policy's time slice and count, and each caller gets
//! its answer after the one commit. A read in the mailbox (a phone's view, `queue show`) does not
//! end the collection: it reads the committed world, which does not change before the batch is
//! drained, so it is answered at once and the batch goes on. Only a command that changes
//! something ends it.

use std::num::NonZeroU128;
use std::sync::mpsc::Receiver;

use df_table::envelope::{
    EngineInput, InputEnvelope, PlayerInput, ProposalOrigin, ProposerRef, TimerName, UtteranceId,
    ViewStamp,
};
use df_table::event::TableEvent;
use df_table::identity::{
    Correlation, MachineId, OperationId, RecoveryEpoch, SeatId, SessionRevision, TableScope,
};
use df_table::outcome::DeclineCode;
use df_table::owner_queue::{AdmitError, NOT_YET_SERVED, OperationStatus, RefusalReport};
use df_table::ports::ActionId;
use df_table::private_data::PlayerText;

use crate::engine::audit::{EngineAudit, SubmitResult};
use crate::engine::labels::sender_label;
use crate::engine::limits::MAXIMUM_RECENT_SUBMISSIONS;
use crate::engine::report::{
    EngineError, RefusalCause, RefusalReport as RefusalView, SubmissionReport, SubmitReply,
    TurnReport,
};
use crate::engine::request::{InputBody, InputRequest};

use super::{Command, Owner, Reply, STATUS_IN_PROGRESS, STATUS_QUEUED, answer};

fn status_text(status: OperationStatus) -> String {
    match status {
        OperationStatus::Queued { lane } => format!("{STATUS_QUEUED} lane={}", lane.key()),
        OperationStatus::InProgress { turn_seq } => {
            format!("{STATUS_IN_PROGRESS} turn_seq={turn_seq}")
        }
        OperationStatus::Finished { turn_seq, outcome } => {
            format!("finished turn_seq={turn_seq} outcome={}", outcome.key())
        }
        OperationStatus::Abandoned { turn_seq } => format!("abandoned turn_seq={turn_seq}"),
    }
}

fn invalid_name(text: &str) -> EngineError {
    EngineError::InvalidName {
        what: text.to_owned(),
    }
}

/// The line a seat reads when its input was aimed at a machine that a hand-off replaced.
const STALE_MACHINE_DETAIL: &str = "That offer belongs to a scene that has moved on.";

/// A submit that was admitted or refused, waiting for its turns and its answer.
type Waiting = (SubmissionReport, Reply<SubmitReply>);

impl Owner {
    /// A submit to an owner that is holding: it is admitted and nothing is served.
    pub(super) fn submit_held(
        &mut self,
        request: InputRequest,
    ) -> Result<SubmitReply, EngineError> {
        let submission = self.admit_request(request)?;
        Ok(SubmitReply {
            submission,
            turns: Vec::new(),
        })
    }

    /// Collects `first` and every submit waiting behind it into one batch, serves it, writes one
    /// commit, and answers each. A read is answered where it stands; the first command that
    /// changes something ends the collection and is returned to be handled next.
    pub(super) fn serve_submits(
        &mut self,
        first: (InputRequest, Reply<SubmitReply>),
        mailbox: &Receiver<Box<Command>>,
    ) -> Option<Command> {
        let started_ms = self.now_ms();
        let mut waiting: Vec<Waiting> = Vec::new();
        let mut carried = None;
        let mut next = Some(first);
        while let Some((request, reply)) = next.take() {
            match self.admit_request(request) {
                Ok(report) => waiting.push((report, reply)),
                Err(error) => answer(reply, Err(error)),
            }
            if !self.may_collect_more(started_ms, waiting.len()) {
                break;
            }
            next = self.next_submit(mailbox, &mut carried, started_ms, waiting.len());
        }
        let turns = self.drain();
        let shares = partition_turns(turns, &waiting);
        for ((submission, reply), turns) in waiting.into_iter().zip(shares) {
            answer(reply, Ok(SubmitReply { submission, turns }));
        }
        carried
    }

    /// The next submit waiting in the mailbox, answering the reads in front of it. `None` when the
    /// mailbox is empty or closed (nothing more waits), when a command that changes something comes
    /// first (it is left in `carried`), or when the batch is full.
    fn next_submit(
        &mut self,
        mailbox: &Receiver<Box<Command>>,
        carried: &mut Option<Command>,
        started_ms: u64,
        collected: usize,
    ) -> Option<(InputRequest, Reply<SubmitReply>)> {
        while self.may_collect_more(started_ms, collected) {
            match *mailbox.try_recv().ok()? {
                Command::Submit(request, reply) => return Some((request, reply)),
                read if read.only_reads() => self.handle(read),
                other => {
                    *carried = Some(other);
                    return None;
                }
            }
        }
        None
    }

    /// Whether the batch may take another submit: under the policy's count and inside its slice.
    fn may_collect_more(&self, started_ms: u64, collected: usize) -> bool {
        let policy = self.config.policy;
        let within_slice = policy
            .time_slice_ms()
            .is_none_or(|slice_ms| self.now_ms().saturating_sub(started_ms) < slice_ms);
        collected < policy.maximum_inputs() && within_slice
    }

    /// Builds the envelope and offers it to the queue. Only an invalid request is an `Err`; a
    /// refusal by the queue is the report.
    fn admit_request(&mut self, request: InputRequest) -> Result<SubmissionReport, EngineError> {
        let envelope = self.envelope(&request)?;
        let now_ms = self.now_ms();
        let (sender, operation) = (envelope.sender(), envelope.operation_id());
        let kind = envelope.kind().key();
        let mut report = SubmissionReport {
            operation: operation.to_string(),
            sender: sender_label(sender),
            kind: kind.to_owned(),
            lane: None,
            refusal: None,
        };
        let admitted = if self.aimed_at_replaced_machine(&request, &envelope) {
            self.refuse_stale_machine(&envelope);
            report.refusal = Some(RefusalView {
                cause: RefusalCause::Declined(DeclineCode::WindowClosed),
                detail: STALE_MACHINE_DETAIL.to_owned(),
                retry_after_ms: None,
                first_status: None,
            });
            false
        } else if let Some(retry_after_ms) = self.backoff_remaining_ms(now_ms) {
            self.refuse_while_backing_off(&envelope, retry_after_ms);
            report.refusal = Some(RefusalView {
                cause: RefusalCause::Declined(DeclineCode::TableBusy),
                detail: "the table is recovering from a failed save".to_owned(),
                retry_after_ms: Some(retry_after_ms),
                first_status: None,
            });
            false
        } else {
            match self.queue.admit(envelope, now_ms) {
                Ok(admitted) => {
                    report.lane = Some(admitted.lane);
                    true
                }
                Err(error) => {
                    report.refusal = Some(refusal_report(&error));
                    false
                }
            }
        };
        self.audit(EngineAudit::Input {
            sender,
            kind,
            operation,
            result: if admitted {
                SubmitResult::Admitted
            } else {
                SubmitResult::Refused
            },
        });
        self.remember_submission(report.clone());
        Ok(report)
    }

    fn remember_submission(&mut self, report: SubmissionReport) {
        self.recent_submissions.push_back(report);
        while self.recent_submissions.len() > MAXIMUM_RECENT_SUBMISSIONS {
            self.recent_submissions.pop_front();
        }
    }

    /// Whether the request names a machine and the table is running another: a hand-off replaced
    /// the machine the offer was drawn from. A repeat of an operation the queue already knows is
    /// the queue's to answer, whatever machine runs now, so it is never stale.
    fn aimed_at_replaced_machine(&self, request: &InputRequest, envelope: &InputEnvelope) -> bool {
        let Some(aimed) = request.machine.as_deref() else {
            return false;
        };
        if self
            .queue
            .status(envelope.sender(), envelope.operation_id())
            .is_some()
        {
            return false;
        }
        self.committed
            .facts()
            .snapshot()
            .focus_machine()
            .is_none_or(|running| running.machine().as_str() != aimed)
    }

    /// Records the decline of an input aimed at a machine that no longer runs, without admitting
    /// it: the operation id stays free.
    fn refuse_stale_machine(&self, envelope: &InputEnvelope) {
        let correlation = Correlation {
            operation_id: envelope.operation_id(),
            turn_seq: NOT_YET_SERVED,
            revision: self.session_revision(),
        };
        self.sink.record(&TableEvent::InputDeclined {
            scope: TableScope::of_table(self.config.table),
            correlation,
            code: DeclineCode::WindowClosed,
        });
    }

    fn refuse_while_backing_off(&self, envelope: &InputEnvelope, retry_after_ms: u64) {
        let correlation = Correlation {
            operation_id: envelope.operation_id(),
            turn_seq: NOT_YET_SERVED,
            revision: self.session_revision(),
        };
        self.sink.record(&TableEvent::InputDeclined {
            scope: TableScope::of_table(self.config.table),
            correlation,
            code: DeclineCode::TableBusy,
        });
        self.audit(EngineAudit::BackingOff {
            operation: envelope.operation_id(),
            retry_after_ms,
        });
    }

    fn operation_for(&mut self, request: &InputRequest) -> Result<OperationId, EngineError> {
        let wide = request.operation_id.or(request.operation.map(u128::from));
        match wide {
            Some(number) => NonZeroU128::new(number)
                .map(OperationId::from_nonzero_u128)
                .ok_or(EngineError::ZeroOperation),
            None => Ok(match request.input {
                InputBody::Timer { .. } | InputBody::Proposal { .. } => {
                    self.mint_engine_operation()
                }
                _ => self.mint_seat_operation(),
            }),
        }
    }

    fn next_utterance(&mut self) -> UtteranceId {
        self.next_utterance = self.next_utterance.saturating_add(1);
        UtteranceId::new(self.next_utterance)
    }

    fn envelope(&mut self, request: &InputRequest) -> Result<InputEnvelope, EngineError> {
        let operation = self.operation_for(request)?;
        let view = ViewStamp::new(match request.view {
            Some(sequence) => SessionRevision::new(RecoveryEpoch::FIRST, sequence),
            None => self.session_revision(),
        });
        let from_seat = |seat: u64, input: PlayerInput| {
            InputEnvelope::from_player(operation, SeatId::new(seat), view, input)
        };
        Ok(match &request.input {
            InputBody::Tap { seat, action } => from_seat(
                *seat,
                PlayerInput::Tap {
                    action: ActionId::new(action).map_err(|_| invalid_name(action))?,
                },
            ),
            InputBody::Text { seat, text } => {
                let utterance = self.next_utterance();
                from_seat(
                    *seat,
                    PlayerInput::SubmitText {
                        utterance,
                        text: PlayerText::new(text.clone()),
                    },
                )
            }
            InputBody::Say { seat, text } => {
                let utterance = self.next_utterance();
                from_seat(
                    *seat,
                    PlayerInput::Say {
                        utterance,
                        text: PlayerText::new(text.clone()),
                    },
                )
            }
            InputBody::Private { seat, to, text } => {
                let utterance = self.next_utterance();
                from_seat(
                    *seat,
                    PlayerInput::PrivateMessage {
                        utterance,
                        to: SeatId::new(*to),
                        text: PlayerText::new(text.clone()),
                    },
                )
            }
            InputBody::Pause { seat } => from_seat(*seat, PlayerInput::SafetyCard),
            InputBody::Timer { name } => InputEnvelope::from_engine(
                operation,
                view,
                EngineInput::Timer {
                    timer: TimerName::new(name).map_err(|_| invalid_name(name))?,
                },
            ),
            InputBody::Proposal { origin, proposer } => InputEnvelope::from_engine(
                operation,
                view,
                EngineInput::Proposal {
                    origin: origin.origin(),
                    proposer: ProposerRef::new(proposer).map_err(|_| invalid_name(proposer))?,
                },
            ),
        })
    }

    /// Asks the queue for the hand-off a finished machine wants: an engine proposal in the engine
    /// lane, served before any player input (RES21). `false` when the queue refused it, which it
    /// has already recorded.
    pub(super) fn queue_hand_off(&mut self, machine: &MachineId) -> bool {
        let Ok(proposer) = ProposerRef::new(machine.as_str()) else {
            return false;
        };
        let operation = self.mint_engine_operation();
        let envelope = InputEnvelope::from_engine(
            operation,
            ViewStamp::new(self.session_revision()),
            EngineInput::Proposal {
                origin: ProposalOrigin::Machine,
                proposer,
            },
        );
        let now_ms = self.now_ms();
        self.queue.admit(envelope, now_ms).is_ok()
    }

    /// Turns every resource request that settled into an engine input (readiness is pushed to the
    /// owner, RES22).
    pub(super) fn admit_resource_results(&mut self, now_ms: u64) {
        for completion in self.adapters.broker.hub.take_completions() {
            let operation = self.mint_engine_operation();
            let envelope = InputEnvelope::from_engine(
                operation,
                ViewStamp::new(self.session_revision()),
                EngineInput::ResourceResult {
                    handle: completion.handle,
                    readiness: completion.readiness,
                },
            );
            // The queue records a refusal itself, and the request has settled either way.
            let _ = self.queue.admit(envelope, now_ms);
        }
    }

    /// How much longer the owner refuses input, or `None` when it is not backing off.
    pub(super) fn backoff_remaining_ms(&self, now_ms: u64) -> Option<u64> {
        self.backoff_until_ms
            .map(|until| until.saturating_sub(now_ms))
            .filter(|remaining| *remaining > 0)
    }
}

/// Gives each submit the turns that ran for its own input, and the engine's follow-ups (a
/// hand-off it asked for) that ran right after. Turns of inputs nobody here sent, such as a
/// resource result, belong to no answer and are listed by `queue show`.
fn partition_turns(turns: Vec<TurnReport>, waiting: &[Waiting]) -> Vec<Vec<TurnReport>> {
    let mut shares: Vec<Vec<TurnReport>> = waiting.iter().map(|_| Vec::new()).collect();
    let mut current: Option<usize> = None;
    for turn in turns {
        let own = waiting.iter().position(|(report, _)| {
            report.refusal.is_none()
                && report.operation == turn.operation
                && report.sender == turn.sender
        });
        match own {
            Some(index) => current = Some(index),
            None if turn.sender == "engine" => {}
            None => current = None,
        }
        if let Some(share) = current.and_then(|index| shares.get_mut(index)) {
            share.push(turn);
        }
    }
    shares
}

fn refusal_report(error: &AdmitError) -> RefusalView {
    let cause = match error.report() {
        RefusalReport::Declined(code) => RefusalCause::Declined(code),
        RefusalReport::NotDeclined(cause) => RefusalCause::NotDeclined(cause),
    };
    RefusalView {
        cause,
        detail: error.to_string(),
        retry_after_ms: match error {
            AdmitError::RateLimited { retry_after_ms } => Some(*retry_after_ms),
            _ => None,
        },
        first_status: match error {
            AdmitError::Duplicate { first } => Some(status_text(*first)),
            _ => None,
        },
    }
}
