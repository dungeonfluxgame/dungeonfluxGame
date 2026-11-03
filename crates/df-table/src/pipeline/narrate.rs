//! Stage 5, NARRATE: the storyteller's flavour is a job, never a wait (C, C.7, H.6).
//!
//! The structured readout of the committed outcome is already the table's answer. NARRATE stages a
//! `NarrationJob` (the correlation and the committed outcome with every applied step and its typed
//! facts) in the turn's result; the host runs it off the owner thread after its commit, with the
//! `Narrator` port, and gives the answer back to `Pipeline::attach_narration`, which qualifies it
//! and appends it after the readout. The pipeline never calls the narrator and never blocks on a
//! model, so a batch of inputs does not wait for text and a refused group commit has spent no
//! narration.
//!
//! `attach_narration` records each failure once, here, with a closed cause: a narrator that failed
//! (its own cause), text that is empty or longer than `maximum_narration_chars` (`text_invalid`),
//! and text that arrives later than `narration_late_s` after the readout was shown (`timed_out`: it
//! would land after the table moved on, and it is skipped, never replacing the readout). What the
//! pipeline can check is the shape of the text. That it states nothing a player character does,
//! says or feels beyond the committed outcome, and nothing a seat may not know, is qualified by
//! the narrator's adapter before it hands the text over (C.5, H.6); the pipeline never searches
//! the words. A qualified line extends the public story line the turn already pushed with its
//! structured readout (`NarrationJob::story_sequence`), so the story holds one line per turn and
//! the readout the seat saw is the readout it keeps; a line the bounded story has dropped is not
//! brought back, and a turn that pushed none gets one line of its own.

use crate::event::{FailureCause, PipelineStep, TableEvent};
use crate::identity::{Correlation, SeatId};
use crate::outcome::OutcomeKind;
use crate::ports::{CommittedOutcome, PortError, PortName};
use crate::private_data::{Measured, Narration};
use crate::projection::{NonEmptyText, StoryLine};

use super::Pipeline;
use super::events::PipelineEvent;
use super::record::applied_line;
use super::table_access::PipelineTable;
use super::turn::Turn;

const MILLISECONDS_PER_SECOND: u64 = 1_000;

/// The narration an applied turn asks for: what the host gives the narrator after its commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NarrationJob {
    pub correlation: Correlation,
    pub committed: CommittedOutcome,
    /// The place in the public story of the line this turn pushed with its readout, which the
    /// narration extends. `None` when the turn pushed no line.
    pub story_sequence: Option<u64>,
}

/// Stages the narration of an applied turn. It calls nothing and waits for nothing.
pub(super) fn stage(turn: &mut Turn<'_, '_>, seat: SeatId) {
    let started = turn.now();
    if let Some((_, action)) = &turn.acted {
        turn.narration = Some(NarrationJob {
            correlation: turn.correlation,
            committed: CommittedOutcome {
                seat,
                action: action.clone(),
                outcome: OutcomeKind::Applied,
                steps: turn.applied_steps.clone(),
                facts: turn.result_facts.clone(),
            },
            story_sequence: None,
        });
    }
    turn.stage_done(PipelineStep::Narrate, started, None);
}

impl Pipeline<'_> {
    /// Qualifies the narrator's answer to a job and appends it after the readout, or records why
    /// not. `shown_at_ms` is the clock reading when the structured readout was shown. Returns the
    /// qualified narration, if there is one.
    pub fn attach_narration(
        &self,
        table: &mut PipelineTable,
        job: &NarrationJob,
        shown_at_ms: u64,
        answer: Result<Narration, PortError>,
    ) -> Option<Narration> {
        let waited_ms = self.ports.clock.now_ms().saturating_sub(shown_at_ms);
        let scope = table.scope();
        let fail = |port: PortName, cause: FailureCause| {
            self.ports.sink.record(&TableEvent::BoundaryFailed {
                scope: scope.clone(),
                correlation: job.correlation,
                port,
                cause,
                ms: waited_ms,
            });
        };
        let narration = match answer {
            Ok(narration) => narration,
            Err(error) => {
                fail(error.port, error.cause);
                return None;
            }
        };
        let late_ms = u64::from(self.settings.narration_late_s()) * MILLISECONDS_PER_SECOND;
        if waited_ms > late_ms {
            fail(PortName::Narrator, FailureCause::TimedOut);
            return None;
        }
        let text = narration.expose();
        if text.trim().is_empty() || text.length_in_chars() > self.limits.maximum_narration_chars {
            fail(PortName::Narrator, FailureCause::TextInvalid);
            return None;
        }
        let committed = &job.committed;
        let attached = match job.story_sequence {
            Some(sequence) => table
                .attachments_mut()
                .extend_public_narration(sequence, narration.clone()),
            None => {
                let readout = NonEmptyText::or_fallback(&applied_line(
                    committed.seat,
                    &committed.action,
                    &committed.facts,
                ));
                let line = StoryLine::new(readout).with_narration(narration.clone());
                table.attachments_mut().push_public(line).map(|()| true)
            }
        };
        if !matches!(attached, Ok(true)) {
            fail(PortName::Narrator, FailureCause::Unavailable);
            return None;
        }
        self.ports
            .sink
            .record(&TableEvent::Pipeline(PipelineEvent::NarrationAttached {
                scope,
                correlation: job.correlation,
                ms: waited_ms,
            }));
        Some(narration)
    }
}
