//! The dev verb `rules check <skill> <actor> [dc=N]`: one real check through the production rules
//! adapter with a recorded draw.
//!
//! It owns the verb's grammar and usage, its typed request and report, the numbered text of the
//! answer, the audit line and the `RulesCheckKit` that runs a check. The kit does what the pipeline
//! does for one action (ask the adapter what it needs, draw each die from the recording dice, ask
//! it to resolve) and nothing else: it reads no rule itself, so the answer is exactly what the
//! adapter's port methods produce. It never writes table state. A call that cannot be answered is
//! an `EngineError` (the port that failed is named), and a decline is a normal report.

use std::num::NonZeroU128;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use df_table::identity::{
    Correlation, OperationId, RecoveryEpoch, SeatId, SessionRevision, TurnSeq,
};
use df_table::outcome::DeclineCode;
use df_table::ports::{Dice, Preparation, Resolution};
use serde::{Deserialize, Serialize};

use crate::command::CommandError;
use crate::engine::report::EngineError;

use super::action_book::{DifficultyClass, RulesEntry};
use super::dice::RecordedDice;
use super::hero_profile::{Ability, Skill};
use super::rules::DfRulesOwner;

pub const RULES_CHECK_USAGE: &str = "rules check <skill> <seat> [dc=N]";

/// The operation ids of dev checks start here, far above the small numbers the owner's inputs
/// use, so a recorded dev draw can never be mistaken for an input's.
const DEV_CHECK_OPERATION_BASE: u128 = 1 << 100;

/// What `rules check` asks for. The actor is a seat, which is how the rules port names a hero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulesCheckRequest {
    pub skill: Skill,
    pub actor: u64,
    /// The authored difficulty class; the SRD's Medium (15) when absent.
    pub dc: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedFact {
    pub label: String,
    pub amount: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedDraw {
    pub sequence: u64,
    pub sides: u16,
    pub value: u16,
}

/// The answer to `rules check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum RulesCheckReport {
    Resolved {
        skill: Skill,
        ability: Ability,
        actor: u64,
        difficulty_class: u8,
        succeeded: bool,
        facts: Vec<ReportedFact>,
        draws: Vec<ReportedDraw>,
    },
    Declined {
        actor: u64,
        code: DeclineCode,
        reason: String,
        missing_piece: Option<String>,
    },
}

fn usage() -> CommandError {
    CommandError::Usage(RULES_CHECK_USAGE)
}

/// Parses the words after `rules`.
pub fn parse(words: &[String]) -> Result<RulesCheckRequest, CommandError> {
    let [verb, skill, actor, rest @ ..] = words else {
        return Err(usage());
    };
    if verb != "check" {
        return Err(usage());
    }
    let skill = Skill::from_key(skill).ok_or_else(|| CommandError::BadChoice {
        key: "the skill".to_owned(),
        text: skill.clone(),
        allowed: Skill::ALL.map(Skill::key).join(", "),
    })?;
    let actor = actor.parse::<u64>().map_err(|_| CommandError::BadCount {
        what: "the seat",
        text: actor.clone(),
    })?;
    let dc = match rest {
        [] => None,
        [pair] => {
            let text = pair.strip_prefix("dc=").ok_or_else(usage)?;
            Some(text.parse::<u8>().map_err(|_| CommandError::BadCount {
                what: "the difficulty class",
                text: text.to_owned(),
            })?)
        }
        _ => return Err(usage()),
    };
    Ok(RulesCheckRequest { skill, actor, dc })
}

fn port_failed(error: df_table::ports::PortError) -> EngineError {
    EngineError::NotAvailable {
        what: error.to_string(),
    }
}

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
    pub fn run(&self, request: &RulesCheckRequest) -> Result<RulesCheckReport, EngineError> {
        let difficulty_class = match request.dc {
            Some(value) => {
                DifficultyClass::new(value).map_err(|error| EngineError::BadRequest {
                    reason: error.to_string(),
                })?
            }
            None => DifficultyClass::MEDIUM,
        };
        let seat = SeatId::new(request.actor);
        let entry = RulesEntry::skill_check(request.skill, difficulty_class);
        let correlation = self.next_correlation();
        let requests = match self
            .rules
            .prepare_entry(seat, &entry)
            .map_err(port_failed)?
        {
            Preparation::Ready { draws } => draws,
            Preparation::Declined(decline) => return Ok(declined(request.actor, &decline)),
        };
        let mut drawn = Vec::with_capacity(requests.len());
        for requested in requests {
            drawn.push(
                self.dice
                    .draw(correlation, requested)
                    .map_err(port_failed)?,
            );
        }
        let resolved = match self
            .rules
            .resolve_entry(seat, &entry, &drawn)
            .map_err(port_failed)?
        {
            Resolution::Applied(resolved) => resolved,
            Resolution::Declined(decline) => return Ok(declined(request.actor, &decline)),
            Resolution::NeedsDraws(_) | Resolution::Await(_) => {
                return Err(EngineError::NotAvailable {
                    what: "the rules adapter asked for more than a check needs".to_owned(),
                });
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
            .ok_or_else(|| EngineError::NotAvailable {
                what: "the rules adapter reported no verdict".to_owned(),
            })?;
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

/// The numbered text of an answer.
pub fn format_rules_check(report: &RulesCheckReport) -> String {
    let lines = match report {
        RulesCheckReport::Resolved {
            skill,
            ability,
            actor,
            difficulty_class,
            succeeded,
            facts,
            draws,
        } => {
            let verdict = if *succeeded { "success" } else { "failure" };
            let head = format!(
                "{} ({}) check for seat {actor} against DC {difficulty_class}: {verdict}",
                skill.key(),
                ability.key()
            );
            let facts = facts
                .iter()
                .map(|fact| format!("{}={}", fact.label, fact.amount))
                .collect::<Vec<_>>()
                .join(" ");
            std::iter::once(head)
                .chain(std::iter::once(facts))
                .chain(draws.iter().map(|draw| {
                    format!(
                        "recorded draw {}: d{} = {}",
                        draw.sequence, draw.sides, draw.value
                    )
                }))
                .collect::<Vec<_>>()
        }
        RulesCheckReport::Declined {
            actor,
            code,
            reason,
            missing_piece,
        } => {
            let missing = missing_piece
                .as_ref()
                .map_or(String::new(), |piece| format!(" Missing: {piece}."));
            vec![format!(
                "seat {actor} declined ({}): {reason}{missing}",
                code.key()
            )]
        }
    };
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| format!("{}: {line}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The audit line of a check: closed keys and numbers only.
pub fn audit_line(request: &RulesCheckRequest, report: &RulesCheckReport) -> String {
    let result = match report {
        RulesCheckReport::Resolved {
            succeeded: true, ..
        } => "success",
        RulesCheckReport::Resolved {
            succeeded: false, ..
        } => "failure",
        RulesCheckReport::Declined { code, .. } => code.key(),
    };
    format!(
        "ENGINE rules_check skill={} seat={} result={result}",
        request.skill.key(),
        request.actor
    )
}
