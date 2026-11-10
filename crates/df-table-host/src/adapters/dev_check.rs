//! The dev verbs `rules check <skill> <actor> [dc=N]` and `rules roll <kind> ..`: one real check,
//! or any typed roll, through the production rules adapter with recorded draws.
//!
//! It owns the `RulesCheckKit` that runs them; its children own the verbs' grammar, usage and
//! calls (`cli`), their wire values and typed refusal (`request`), and the numbered text of the
//! answers and the audit lines (`format`). The routes are `dev_route`. The kit does what the
//! pipeline does for one action (ask the adapter what it needs, draw each die from the recording
//! dice, ask it to resolve) and nothing else: it reads no rule itself, so the answer is exactly
//! what the adapter produces. It never writes table state. A call that cannot be answered is a
//! `RulesCheckError` (the port that failed, or the roll `df-rules` refused, is named), and a
//! decline is a normal report.

mod cli;
mod format;
mod request;

use std::num::NonZeroU128;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use df_rules::roll::RollRequest;
use df_table::identity::{
    Correlation, OperationId, RecoveryEpoch, SeatId, SessionRevision, TurnSeq,
};
use df_table::ports::{ActionId, Dice, Preparation, Resolution};

use super::action_book::{DifficultyClass, RulesEntry};
use super::dice::RecordedDice;
use super::rules::{DfRulesOwner, RollStep, prepare_roll, resolve_roll, rolled_line};

pub use self::cli::{
    RULES_CHECK_USAGE, RULES_ROLL_USAGE, ask, ask_roll, parse, parse_roll, run, run_roll,
};
pub use self::format::{
    audit_line, format_rules_check, format_rules_roll, refusal_audit_line, roll_audit_line,
    roll_refusal_audit_line,
};
pub use self::request::{
    ReportedDraw, ReportedFact, RulesCheckError, RulesCheckReply, RulesCheckReport,
    RulesCheckRequest, RulesRollReply, RulesRollReport,
};

/// The operation ids of dev checks start here, far above the small numbers the owner's inputs
/// use, so a recorded dev draw can never be mistaken for an input's.
const DEV_CHECK_OPERATION_BASE: u128 = 1 << 100;

/// The action name the audit lines of a dev roll carry.
const DEV_ROLL_ACTION: &str = "dev_roll";

/// Runs checks through the production adapter and its recording dice.
pub struct RulesCheckKit {
    rules: Arc<DfRulesOwner>,
    dice: Arc<RecordedDice>,
    next_operation: AtomicU64,
    roll_action: ActionId,
}

impl RulesCheckKit {
    pub fn new(rules: Arc<DfRulesOwner>, dice: Arc<RecordedDice>) -> Self {
        Self {
            rules,
            dice,
            next_operation: AtomicU64::new(0),
            roll_action: ActionId::new(DEV_ROLL_ACTION)
                .expect("the dev roll action is a valid action name"),
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

    /// Asks the adapter for the dice a roll needs, draws each from the recording dice and
    /// resolves the roll with them.
    pub fn roll(&self, request: &RollRequest) -> Result<RulesRollReport, RulesCheckError> {
        request
            .draw_plan()
            .map_err(|cause| RulesCheckError::InvalidRoll { cause })?;
        let correlation = self.next_correlation();
        let operation = correlation.operation_id;
        let mut drawn = Vec::new();
        for requested in prepare_roll(&self.roll_action, operation, request)? {
            drawn.push(self.dice.draw(correlation, requested)?);
        }
        let result = match resolve_roll(&self.roll_action, operation, request, &drawn)? {
            RollStep::Rolled(result) => *result,
            RollStep::NeedsDraws(_) => return Err(RulesCheckError::UnexpectedResolution),
        };
        let audit = vec![rolled_line(&self.roll_action, operation, &result)];
        Ok(RulesRollReport {
            result,
            draws: self.recorded(correlation),
            audit,
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
