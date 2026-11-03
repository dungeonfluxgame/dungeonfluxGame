//! Stage 6, PROJECT: how an input ends, the line the seat is shown, and one projection per audience
//! (C step 6, C.2, C.5, C.7, K.5, H.15).
//!
//! `finish` is where every input ends, fresh or replayed. It stages the lines that say what the
//! input came to (a decline, a clarification, a routed line: `outcome_events`, released with the
//! turn and never before), builds the structured readout from typed fields and the rules owner's
//! decline text, remembers how the input ended (the result book if it committed, the session's
//! memory of uncommitted ends if not) so a duplicate delivery is answered without a port, and then
//! publishes to the attachments the projections read: the readouts, the notice, the story line
//! and the activity, and the buttons every seat holds (`offers`). A duplicate delivery writes
//! nothing: it is only projected, and the seat is told what it was told the first time. A plan
//! that stopped at a step after a valid prefix was applied asks the one question about that step
//! (C.2, RES26), with the seat's legal choices as its buttons.
//!
//! A part of the views that is refused (a text over the wire frame, a bundle at its bound, a
//! projection that cannot be built) is recorded once with its part (`PROJECT view_failed`) and
//! leaves the views at what they showed with a `degraded` status; the turn is not failed by it. It
//! never searches the words of a line and writes no rules text.

use crate::envelope::PendingId;
use crate::event::{PipelineStep, TableEvent};
use crate::identity::{SeatId, SessionRevision};
use crate::outcome::{Decline, Outcome, PendingKind};
use crate::ports::Audience;
use crate::projection::{
    AttachmentError, Attachments, AudienceProjection, NonEmptyText, ProjectionStatus, StoryLine,
    project,
};

use super::events::{ClarifyCause, PipelineEvent, ViewPart};
use super::record::{self, applied_line, routed_line};
use super::stored::stored_outcome;
use super::table_access::PipelineTable;
use super::turn::Turn;
use super::{Clarification, Pipeline, Readout, TurnRecord, TurnResult, offers};

impl Pipeline<'_> {
    /// PROJECT: the structured readout, the projections, the lines that explain the end, and the
    /// record.
    pub(super) fn finish(
        &self,
        mut turn: Turn<'_, '_>,
        table: &mut PipelineTable,
        outcome: Outcome,
    ) -> TurnResult {
        let started = turn.now();
        let outcome = if turn.replayed {
            outcome
        } else {
            self.record_end(&mut turn, table, outcome)
        };
        let text = match turn.replayed_readout.clone() {
            Some(shown) => shown,
            None => self.readout_text(&turn, table, &outcome),
        };
        let readout = Readout::new(text);
        if !turn.replayed {
            let stored = stored_outcome(&turn, &outcome, readout.as_str());
            let key = turn.result_key();
            if turn.committed {
                table.results_mut().record(key, turn.kind, stored);
            } else {
                table.ends_mut().record(key, turn.kind, stored);
            }
        }
        let projections = self.project_audiences(&mut turn, table, &outcome, &readout);
        turn.stage_done(PipelineStep::Project, started, Some(outcome.kind()));
        let declined = match &outcome {
            Outcome::Declined(decline) => Some(decline.code()),
            Outcome::Applied | Outcome::NeedsClarification | Outcome::Pending(_) => None,
        };
        let record = TurnRecord {
            correlation: turn.correlation,
            sender: turn.sender,
            kind: turn.kind,
            outcome: outcome.kind(),
            declined,
            pending: turn.pending,
            draws: turn.draws,
            effects: turn.effects,
            state_hash_before: turn.hash_before,
            state_hash_after: table.state_hash(),
            durable_hash_before: turn.durable_before,
            durable_hash_after: table.durable_hash(),
            replayed: turn.replayed,
        };
        TurnResult {
            record,
            outcome,
            readout,
            narration_job: turn.narration,
            clarification: turn.clarification,
            pending: turn.pending,
            unfinished: turn.unfinished,
            routed: turn.routed,
            hand_off_wanted: turn.hand_off_wanted,
            pause_requested: turn.pause_requested,
            projections,
            outcome_events: turn.outcome_events,
            committed_events: if turn.committed {
                turn.on_commit
            } else {
                Vec::new()
            },
        }
    }

    /// Stages how the input ended, once: a decline, a clarification, a routed line. These say what
    /// was asked of the ports and what the seat was told; they are released with the turn.
    fn record_end(
        &self,
        turn: &mut Turn<'_, '_>,
        table: &mut PipelineTable,
        outcome: Outcome,
    ) -> Outcome {
        if let Some(kind) = turn.routed {
            turn.stage_outcome(turn.pipeline_event(|scope, correlation| {
                PipelineEvent::InputRouted {
                    scope,
                    correlation,
                    kind,
                }
            }));
        }
        let outcome = match outcome {
            Outcome::Declined(decline) => {
                turn.stage_outcome(TableEvent::InputDeclined {
                    scope: turn.scope.clone(),
                    correlation: turn.correlation,
                    code: decline.code(),
                });
                Outcome::Declined(decline)
            }
            other => other,
        };
        if matches!(outcome, Outcome::Applied) && turn.unfinished.is_some() {
            // A plan stopped at a step after a valid prefix was applied: the one question about
            // the first step that failed, with what the seat may do instead (C.2, RES26).
            self.ask_about_unfinished_step(turn);
        }
        let asked = turn
            .clarification
            .as_ref()
            .map(|clarification| (clarification.options.clone(), clarification.cause));
        if let Some((options, cause)) = asked {
            let count = options.len();
            turn.stage_outcome(turn.pipeline_event(|scope, correlation| {
                PipelineEvent::InputClarified {
                    scope,
                    correlation,
                    options: count,
                    cause,
                }
            }));
            if let Some(seat) = turn.seat {
                table.set_clarification(seat, options);
            }
        }
        outcome
    }

    /// A decline from the rules owner with only the alternatives the seat's legal choices contain,
    /// from one `legal_choices` call recorded once if it fails (C.2: the decline text, the phone's
    /// choices and the viable list come from the one legal-choice call). A decline that offers
    /// nothing costs no call. The table's own authored declines (a split, a pause) are not passed
    /// through here: their alternatives are the table's words, not the rules owner's.
    pub(in crate::pipeline) fn with_legal_alternatives(
        &self,
        turn: &Turn<'_, '_>,
        decline: Decline,
    ) -> Decline {
        let Some(seat) = turn.seat else {
            return decline;
        };
        if decline.alternatives().is_empty() {
            return decline;
        }
        let started = turn.now();
        match self.ports.rules.legal_choices(turn.correlation, seat) {
            Ok(legal) => {
                let legal: Vec<String> = legal.iter().map(record::action_words).collect();
                decline.keeping_alternatives(|alternative| {
                    legal
                        .iter()
                        .any(|choice| choice.eq_ignore_ascii_case(alternative.trim()))
                })
            }
            Err(error) => {
                turn.boundary_failed(error.port, error.cause, started);
                decline.keeping_alternatives(|_| false)
            }
        }
    }

    /// Gives the seat the buttons for the step a plan stopped at.
    fn ask_about_unfinished_step(&self, turn: &mut Turn<'_, '_>) {
        let Some(seat) = turn.seat else {
            return;
        };
        let started = turn.now();
        match self.ports.rules.legal_choices(turn.correlation, seat) {
            Ok(options) => {
                turn.clarification = Some(Clarification {
                    options,
                    cause: ClarifyCause::StepDeclined,
                    keeps_draft: false,
                });
            }
            Err(error) => turn.boundary_failed(error.port, error.cause, started),
        }
    }

    fn readout_text(
        &self,
        turn: &Turn<'_, '_>,
        table: &PipelineTable,
        outcome: &Outcome,
    ) -> String {
        match outcome {
            Outcome::Declined(decline) => declined_line(decline),
            Outcome::NeedsClarification => match turn.clarification.as_ref().map(|c| c.cause) {
                Some(ClarifyCause::Ambiguous) => "Which did you mean? Pick one:".to_owned(),
                Some(
                    ClarifyCause::Unclear | ClarifyCause::Unreadable | ClarifyCause::StepDeclined,
                )
                | None => "I could not read that. Pick one:".to_owned(),
            },
            Outcome::Pending(kind) => pending_line(table, turn.pending, *kind),
            Outcome::Applied => applied_text(turn),
        }
    }
}

/// The line for a decline: the rules owner's reason, then what would be possible (C.6).
fn declined_line(decline: &Decline) -> String {
    let mut line = decline.reason().to_owned();
    let alternatives = decline.alternatives();
    let Some((last, earlier)) = alternatives.split_last() else {
        return line;
    };
    line.push_str(" You could ");
    match earlier.split_last() {
        None => line.push_str(last),
        Some((only, [])) => line.push_str(&format!("{only} or {last}")),
        Some(_) => {
            for alternative in earlier {
                line.push_str(alternative);
                line.push_str(", ");
            }
            line.push_str(&format!("or {last}"));
        }
    }
    line.push('.');
    line
}

fn applied_text(turn: &Turn<'_, '_>) -> String {
    if let Some(kind) = turn.routed {
        return routed_line(kind).to_owned();
    }
    let mut line = String::new();
    if let Some(answer) = &turn.ruling_note {
        line.push_str(&format!(
            "Ruling (earlier): {}. ",
            answer.as_str().replace(['_', '-'], " ")
        ));
    }
    if turn.cancelled {
        line.push_str("Cancelled.");
    } else if let Some((seat, action)) = &turn.acted {
        line.push_str(&applied_line(*seat, action, &turn.result_facts));
    } else {
        line.push_str("Done.");
    }
    if let Some(decline) = &turn.unfinished {
        line.push(' ');
        line.push_str(&declined_line(decline));
    }
    line
}

fn pending_line(table: &PipelineTable, pending: Option<PendingId>, kind: PendingKind) -> String {
    let waiting_on = pending
        .and_then(|id| table.pending().get(id))
        .and_then(|held| held.window().awaiting().next());
    match (kind, waiting_on) {
        (PendingKind::Ruling, _) => "Needs a ruling.".to_owned(),
        (PendingKind::Choice, Some(seat)) => format!("Waiting for seat {seat} to choose."),
        (PendingKind::Roll, Some(seat)) => format!("Waiting for seat {seat} to roll."),
        (PendingKind::Reaction, Some(seat)) => {
            format!("Waiting for seat {seat} to answer a reaction.")
        }
        (_, None) => "Waiting for an answer.".to_owned(),
    }
}

/// The activity every view shows while the table waits on a person (K.5), or `Ok` again.
fn activity_of(
    table: &PipelineTable,
    outcome: &Outcome,
    pending: Option<PendingId>,
) -> ProjectionStatus {
    let Outcome::Pending(kind) = outcome else {
        return ProjectionStatus::Ok;
    };
    let waiting_on = pending
        .and_then(|id| table.pending().get(id))
        .and_then(|held| held.window().awaiting().next());
    let (who, what) = match (kind, waiting_on) {
        (PendingKind::Ruling, _) => ("the host".to_owned(), "to rule"),
        (PendingKind::Choice, Some(seat)) => (format!("seat {seat}"), "to choose"),
        (PendingKind::Roll, Some(seat)) => (format!("seat {seat}"), "to roll"),
        (PendingKind::Reaction, Some(seat)) => (format!("seat {seat}"), "to answer a reaction"),
        (_, None) => ("the table".to_owned(), "to answer"),
    };
    ProjectionStatus::waiting_for(&who, what).unwrap_or(ProjectionStatus::Degraded)
}

impl Pipeline<'_> {
    /// Publishes what this turn said to the attachments the projections read, gives every seat the
    /// buttons it holds, then projects the shared screen and every seat at the table from the table
    /// as the turn left it. A duplicate delivery writes nothing: it is only projected. An
    /// attachment that is refused (a text over the wire frame, a bundle at its bound) leaves the
    /// views at what they showed with a `degraded` status, which says the table could not show
    /// more, and the failure is recorded once with its part; the turn is not failed by it.
    fn project_audiences(
        &self,
        turn: &mut Turn<'_, '_>,
        table: &mut PipelineTable,
        outcome: &Outcome,
        readout: &Readout,
    ) -> Vec<AudienceProjection> {
        if !turn.replayed {
            let text = NonEmptyText::or_fallback(readout.as_str());
            let activity = activity_of(table, outcome, turn.pending);
            let revision = table.facts().snapshot().revision();
            match publish(
                table.attachments_mut(),
                turn,
                outcome,
                text,
                activity,
                revision,
            ) {
                Ok(sequence) => {
                    if let Some(job) = turn.narration.as_mut() {
                        job.story_sequence = sequence;
                    }
                }
                Err(_) => {
                    self.view_failed(turn, table, ViewPart::Publish, None);
                    // The status is a closed value; if even it is refused the views keep the last.
                    if table
                        .attachments_mut()
                        .set_activity(ProjectionStatus::Degraded)
                        .is_err()
                    {
                        self.view_failed(turn, table, ViewPart::Publish, None);
                    }
                }
            }
            if offers::sync_offers(table).is_err() {
                self.view_failed(turn, table, ViewPart::Offers, None);
            }
        }
        let snapshot = table.facts().snapshot();
        let attachments = table.attachments();
        let mut projections = Vec::new();
        let mut refused = Vec::new();
        let mut keep = |audience: Audience, seat: Option<SeatId>| match project(
            &snapshot,
            attachments,
            audience,
        ) {
            Ok(projection) => projections.push(projection),
            Err(_) => refused.push(seat),
        };
        keep(Audience::Public, None);
        for seat in snapshot.seats().iter() {
            keep(Audience::Seat(*seat), Some(*seat));
        }
        for seat in refused {
            self.view_failed(turn, table, ViewPart::Projection, seat);
        }
        projections
    }

    /// Records, once, that part of the views could not be built.
    fn view_failed(
        &self,
        turn: &Turn<'_, '_>,
        table: &PipelineTable,
        part: ViewPart,
        seat: Option<SeatId>,
    ) {
        self.ports
            .sink
            .record(&TableEvent::Pipeline(PipelineEvent::ViewFailed {
                scope: table.scope(),
                correlation: turn.correlation,
                part,
                seat,
            }));
    }
}

/// Writes this turn's readout, notice and story line to the attachments and returns the place of
/// the story line it added, so the narration extends it and adds no second one.
fn publish(
    attachments: &mut Attachments,
    turn: &Turn<'_, '_>,
    outcome: &Outcome,
    text: NonEmptyText,
    activity: ProjectionStatus,
    revision: SessionRevision,
) -> Result<Option<u64>, AttachmentError> {
    attachments.set_activity(activity)?;
    let mut sequence = None;
    match outcome {
        Outcome::Applied => {
            attachments.set_shared_readout(text.clone())?;
            if let Some(seat) = turn.seat {
                attachments.set_seat_readout(seat, text.clone())?;
            }
            for seat in turn
                .also_told
                .iter()
                .filter(|seat| Some(**seat) != turn.seat)
            {
                attachments.set_seat_readout(*seat, text.clone())?;
            }
            sequence = Some(attachments.push_story(StoryLine::new(text))?);
        }
        Outcome::Declined(decline) => {
            if let Some(seat) = turn.seat {
                attachments.set_notice(seat, decline.clone(), revision)?;
                attachments.set_seat_readout(seat, text)?;
            }
        }
        Outcome::NeedsClarification | Outcome::Pending(_) => {
            if let Some(seat) = turn.seat {
                attachments.set_seat_readout(seat, text)?;
            }
        }
    }
    Ok(sequence)
}
