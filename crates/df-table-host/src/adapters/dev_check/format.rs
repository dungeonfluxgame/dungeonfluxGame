//! The text of a `rules check` answer and its audit lines.

use super::request::{RulesCheckError, RulesCheckReport, RulesCheckRequest};

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

/// The audit line of a refused check, the shape of every engine route's refusal: the error's
/// closed key and how long the call took.
pub fn refusal_audit_line(error: &RulesCheckError, duration_ms: u128) -> String {
    format!(
        "ENGINE refused verb=rules_check error={} duration_ms={duration_ms}",
        error.key()
    )
}
