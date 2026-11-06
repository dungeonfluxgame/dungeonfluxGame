//! The dev verb `rules check <skill> <actor> [dc=N]`: one real check through the production rules
//! adapter with a recorded draw.
//!
//! It owns the `RulesCheckKit` that runs a check; its children own the verb's grammar, usage and
//! call (`cli`), its wire values and typed refusal (`request`), and the numbered text of the
//! answer and the audit lines (`format`). The route is `dev_route`. The kit does what the pipeline
//! does for one action (ask the adapter what it needs, draw each die from the recording dice, ask
//! it to resolve) and nothing else: it reads no rule itself, so the answer is exactly what the
//! adapter's port methods produce. It never writes table state. A call that cannot be answered is
//! a `RulesCheckError` (the port that failed is named), and a decline is a normal report.

mod cli;
mod format;
mod request;

use std::num::NonZeroU128;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use df_table::identity::{
    Correlation, OperationId, RecoveryEpoch, SeatId, SessionRevision, TurnSeq,
};
use df_table::ports::{Dice, Preparation, Resolution};

use super::action_book::{DifficultyClass, RulesEntry};
use super::dice::RecordedDice;
use super::rules::DfRulesOwner;

pub use self::cli::{RULES_CHECK_USAGE, ask, parse, run};
pub use self::format::{audit_line, format_rules_check, refusal_audit_line};
pub use self::request::{
    ReportedDraw, ReportedFact, RulesCheckError, RulesCheckReply, RulesCheckReport,
    RulesCheckRequest,
};

/// The operation ids of dev checks start here, far above the small numbers the owner's inputs
/// use, so a recorded dev draw can never be mistaken for an input's.
const DEV_CHECK_OPERATION_BASE: u128 = 1 << 100;

/// Runs checks through the production adapter and its recording dice.
pub struct RulesCheckKit {
    rules: Arc<DfRulesOwner>,
    dice: Arc<RecordedDice>,
    next_operation: AtomicU64,
}

impl RulesCheckKit {
    pub fn new(rules: Arc<DfRulesOwner>, dice: Arc<RecordedDice>) -> Self {
        Self {
            rules,
            dice,
            next_operation: AtomicU64::new(0),
        }
    }

    fn next_correlation(&self) -> Correlation {
        // Relaxed: the counter only makes ids distinct and orders no other data.
        let number = self.next_operation.fetch_add(1, Ordering::Relaxed);
        let operation =
            NonZeroU128::MIN.saturating_add(DEV_CHECK_OPERATION_BASE + u128::from(number));
        Correlation {
            operation_id: OperationId::from_nonzero_u128(operation),
            turn_seq: TurnSeq::new(0),
            revision: SessionRevision::new(RecoveryEpoch::FIRST, 0),
        }
    }

    /// Asks the adapter what the check needs, draws it, and resolves it.
    pub fn run(&self, request: &RulesCheckRequest) -> Result<RulesCheckReport, RulesCheckError> {
        let difficulty_class = request.dc.unwrap_or(DifficultyClass::MEDIUM);
        let seat = SeatId::new(request.actor);
        let entry = RulesEntry::skill_check(request.skill, difficulty_class);
        let correlation = self.next_correlation();
        let requests = match self.rules.prepare_entry(seat, &entry)? {
            Preparation::Ready { draws } => draws,
            Preparation::Declined(decline) => return Ok(declined(request.actor, &decline)),
        };
        let mut drawn = Vec::with_capacity(requests.len());
        for requested in requests {
            drawn.push(self.dice.draw(correlation, requested)?);
        }
        let resolved = match self.rules.resolve_entry(seat, &entry, &drawn)? {
            Resolution::Applied(resolved) => resolved,
            Resolution::Declined(decline) => return Ok(declined(request.actor, &decline)),
            Resolution::NeedsDraws(_) | Resolution::Await(_) => {
                return Err(RulesCheckError::UnexpectedResolution);
            }
        };
        let facts: Vec<ReportedFact> = resolved
            .facts
            .iter()
            .map(|fact| ReportedFact {
                label: fact.label.as_str().to_owned(),
                amount: fact.amount,
            })
            .collect();
        let succeeded = facts
            .iter()
            .find(|fact| fact.label == "succeeded")
            .map(|fact| fact.amount == 1)
            .ok_or(RulesCheckError::NoVerdict)?;
        let draws = self.recorded(correlation);
        Ok(RulesCheckReport::Resolved {
            skill: request.skill,
            ability: request.skill.ability(),
            actor: request.actor,
            difficulty_class: difficulty_class.get(),
            succeeded,
            facts,
            draws,
        })
    }

    fn recorded(&self, correlation: Correlation) -> Vec<ReportedDraw> {
        self.dice
            .records_of(correlation.operation_id)
            .iter()
            .map(|record| ReportedDraw {
                sequence: record.sequence,
                sides: record.draw.sides().get(),
                value: record.draw.value(),
            })
            .collect()
    }
}

fn declined(actor: u64, decline: &df_table::outcome::Decline) -> RulesCheckReport {
    RulesCheckReport::Declined {
        actor,
        code: decline.code(),
        reason: decline.reason().to_owned(),
        missing_piece: decline.missing_piece().map(str::to_owned),
    }
}
