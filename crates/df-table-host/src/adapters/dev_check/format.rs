//! The text of a `rules check` answer and its audit lines.

use df_rules::roll::{RollRequest, RollSubject};

use super::super::rules::{die_text, subject_key};
use super::request::{RulesCheckError, RulesCheckReport, RulesCheckRequest, RulesRollReport};

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

fn refusal_line(verb: &str, error: &RulesCheckError, duration_ms: u128) -> String {
    format!(
        "ENGINE refused verb={verb} error={} duration_ms={duration_ms}",
        error.key()
    )
}

/// The audit line of a refused check, the shape of every engine route's refusal: the error's
/// closed key and how long the call took.
pub fn refusal_audit_line(error: &RulesCheckError, duration_ms: u128) -> String {
    refusal_line("rules_check", error, duration_ms)
}

/// The audit line of a refused roll, in the same shape.
pub fn roll_refusal_audit_line(error: &RulesCheckError, duration_ms: u128) -> String {
    refusal_line("rules_roll", error, duration_ms)
}

/// The numbered text of a `rules roll` answer: the result, every die, the modifiers, the source
/// clause, the draws the port recorded and the audit lines.
pub fn format_rules_roll(report: &RulesRollReport) -> String {
    let result = &report.result;
    let target = result.target.map_or(String::new(), |target| {
        format!(" against {} {}", target.label.key(), target.value)
    });
    let dice = if result.dice.is_empty() {
        "no dice (a passive score is not rolled)".to_owned()
    } else {
        format!(
            "dice {}",
            result
                .dice
                .iter()
                .map(die_text)
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    let replaced = result
        .inspiration_reroll
        .as_ref()
        .map_or(String::new(), |die| {
            format!(", Heroic Inspiration replaced {}", die_text(die))
        });
    let modifiers = if result.modifiers.is_empty() {
        "modifiers none".to_owned()
    } else {
        format!(
            "modifiers {}",
            result
                .modifiers
                .iter()
                .map(|modifier| format!("{} {:+}", modifier.label.key(), modifier.value))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let mut lines = vec![
        format!(
            "{} {}: {}, total {}{target}",
            subject_key(result.subject),
            result.mode.word().to_lowercase(),
            result.outcome.key(),
            result.total
        ),
        format!("{dice}{replaced}"),
        modifiers,
        format!("natural {}", result.natural.key()),
        format!("source {}", result.subject.source()),
    ];
    lines.extend(report.draws.iter().map(|draw| {
        format!(
            "recorded draw {}: d{} = {}",
            draw.sequence, draw.sides, draw.value
        )
    }));
    lines.extend(report.audit.iter().cloned());
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| format!("{}: {line}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The audit line of a roll the route answered: closed keys only.
pub fn roll_audit_line(request: &RollRequest, report: &RulesRollReport) -> String {
    let kind = match request {
        RollRequest::D20(_) => "d20",
        RollRequest::PassivePerception(_) => "passive_perception",
        RollRequest::Dice(_) => "dice",
    };
    let subject = match report.result.subject {
        RollSubject::Dice(role) => role.key().to_owned(),
        subject => subject_key(subject),
    };
    format!(
        "ENGINE rules_roll kind={kind} subject={subject} outcome={}",
        report.result.outcome.key()
    )
}
