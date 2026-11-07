//! The text each `persona` verb prints: keys and values, never a prompt. The owner view shows
//! everything a persona holds (the dev API is for the operator only).

use df_content::persona::backfill::Gap;
use df_content::persona::{
    Persona, PersonaBehavior, PersonaId, PersonaSecret, PersonaTrigger, PublicPersona,
};

use crate::speech::format_voiced;

use super::request::{
    Answered, BackfillOutcome, BackfillRecord, Backfilled, CastOutcome, Covered, PersonaSummary,
    Spoken,
};

pub(super) fn format_list(list: &[PersonaSummary]) -> String {
    if list.is_empty() {
        return "no personas are stored (store one with: persona write <id> <file>)".to_owned();
    }
    let lines: Vec<String> = list
        .iter()
        .map(|summary| {
            format!(
                "  {}: {}, {}",
                summary.id, summary.display_name, summary.job
            )
        })
        .collect();
    format!("personas:\n{}", lines.join("\n"))
}

/// One line about the persona of `character` and what generating it did.
pub(super) fn format_generation(character: &str, outcome: &CastOutcome) -> String {
    match outcome {
        CastOutcome::Generated {
            persona,
            leak_suspect,
        } => format!(
            "{character}: generated ({}, {}){}",
            persona.display_name,
            persona.job,
            if *leak_suspect {
                "; leak suspect: the model judged its public texts give away its private side"
            } else {
                ""
            }
        ),
        CastOutcome::KeptExisting { persona } => format!(
            "{character}: kept the stored persona ({}, {}); overwrite=true writes a new one",
            persona.display_name, persona.job
        ),
        CastOutcome::Failed { error } => {
            format!("{character}: failed ({}): {error}", error.key())
        }
    }
}

pub(super) fn format_spoken(spoken: &Spoken) -> String {
    let said = &spoken.said;
    let mut lines = vec![format!(
        "ok (revision {}) {} says: {}",
        spoken.revision, spoken.character, said.line
    )];
    if let Some(voice) = &spoken.voice {
        lines.push(format_voiced(voice));
    }
    if said.leak_suspect {
        lines.push(
            "  leak suspect: the model judged the line gives away the private side".to_owned(),
        );
    }
    if let Some(behavior) = &said.behavior {
        lines.push(format!("  behavior: {behavior}"));
    }
    if !said.fired_triggers.is_empty() {
        lines.push(format!(
            "  fired triggers: {}",
            said.fired_triggers.join(", ")
        ));
    }
    if !said.revealed_secrets.is_empty() {
        lines.push(format!(
            "  revealed secrets: {}",
            said.revealed_secrets.join(", ")
        ));
    }
    lines.join("\n")
}

/// The gaps as `field: topic` lines.
fn gap_lines(gaps: &[Gap]) -> Vec<String> {
    gaps.iter()
        .map(|gap| format!("    - {}: {}", gap.field.key(), gap.topic))
        .collect()
}

pub(super) fn format_answered(answered: &Answered) -> String {
    let mut lines = vec![format_spoken(&answered.spoken)];
    lines.push(format!(
        "  covered: {}{}",
        answered.covered,
        if answered.gaps.is_empty() {
            ""
        } else {
            " (gaps below)"
        }
    ));
    lines.extend(gap_lines(&answered.gaps));
    lines.push(match &answered.backfill {
        BackfillOutcome::NotNeeded => "  backfill: not needed".to_owned(),
        BackfillOutcome::NotAsked => "  backfill: not asked (backfill=false)".to_owned(),
        BackfillOutcome::CoverageFailed { error } => {
            format!(
                "  coverage: failed ({}): {error}; answered from the persona as it is",
                error.key()
            )
        }
        BackfillOutcome::Backfilled { edits, sequence } => {
            format!("  backfill: {edits} edits added (record {sequence})")
        }
        BackfillOutcome::BackfillFailed { error } => {
            format!("  backfill: failed ({}): {error}", error.key())
        }
        BackfillOutcome::CoverageSkipped => "  coverage: skipped (coverage=false)".to_owned(),
    });
    lines.push(format!("  model: {}", answered.profile));
    lines.join("\n")
}

pub(super) fn format_covered(covered: &Covered) -> String {
    let mut lines = vec![format!(
        "enough: {} (model {})",
        covered.enough, covered.profile
    )];
    lines.extend(gap_lines(&covered.gaps));
    lines.join("\n")
}

pub(super) fn format_backfilled(character: &PersonaId, done: &Backfilled) -> String {
    let mut lines = vec![format!(
        "ok backfilled {character} (record {}, {} edits, model {})",
        done.sequence,
        done.edits.len(),
        done.profile
    )];
    lines.extend(gap_lines(&done.gaps));
    for edit in &done.edits {
        lines.push(format!(
            "  + {}",
            serde_json::to_string(edit).unwrap_or_else(|_| "(unwritable edit)".to_owned())
        ));
    }
    lines.join("\n")
}

pub(super) fn format_backfills(id: &PersonaId, records: &[BackfillRecord]) -> String {
    if records.is_empty() {
        return format!("no backfills are recorded for {id}");
    }
    let lines: Vec<String> = records
        .iter()
        .map(|record| {
            format!(
                "  {}: at {} ms, {} gaps, {} edits, model {}{}",
                record.sequence,
                record.at_ms,
                record.gaps.len(),
                record.edits.len(),
                record.profile,
                record
                    .question
                    .as_deref()
                    .map_or_else(String::new, |question| format!(", question: {question}"))
            )
        })
        .collect();
    format!("backfills of {id}:\n{}", lines.join("\n"))
}

/// `  label: text`, or `label:` over the indented lines of a text that has several; `None` for a
/// text that is blank.
fn text_block(label: &str, text: &str) -> Option<String> {
    if text.trim().is_empty() {
        None
    } else if text.contains('\n') {
        let lines: Vec<String> = text.lines().map(|line| format!("    {line}")).collect();
        Some(format!("  {label}:\n{}", lines.join("\n")))
    } else {
        Some(format!("  {label}: {text}"))
    }
}

/// `  label:` over one `- entry` line each; `None` for no entries.
fn list_block(label: &str, entries: &[String]) -> Option<String> {
    if entries.is_empty() {
        None
    } else {
        let lines: Vec<String> = entries
            .iter()
            .map(|entry| format!("    - {entry}"))
            .collect();
        Some(format!("  {label}:\n{}", lines.join("\n")))
    }
}

/// `  label:` over the `- line` of each of `items`; `None` for no items.
fn items_block<T>(label: &str, items: &[T], line: impl Fn(&T) -> String) -> Option<String> {
    let lines: Vec<String> = items
        .iter()
        .map(|item| format!("    - {}", line(item)))
        .collect();
    (!lines.is_empty()).then(|| format!("  {label}:\n{}", lines.join("\n")))
}

pub(super) fn format_public(persona: &PublicPersona) -> String {
    let blocks = [
        Some(format!("persona {} (public)", persona.id)),
        text_block("display name", &persona.display_name),
        text_block("job", &persona.job),
        list_block("personality", &persona.personality),
        text_block("speech style", &persona.speech_style),
        text_block("backstory", &persona.backstory),
    ];
    blocks.into_iter().flatten().collect::<Vec<_>>().join("\n")
}

pub(super) fn format_owner(persona: &Persona) -> String {
    let secret = |secret: &PersonaSecret| {
        let mut line = format!(
            "{} [{}]: {}",
            secret.id,
            secret.sensitivity.key(),
            secret.text
        );
        for condition in &secret.reveal_conditions {
            line.push_str(&format!("\n        reveal when: {condition}"));
        }
        if !secret.known_by.is_empty() {
            line.push_str(&format!(
                "\n        known by: {}",
                secret.known_by.join(", ")
            ));
        }
        line
    };
    let behavior = |behavior: &PersonaBehavior| {
        let default = persona.private_default_behavior() == Some(behavior.id.as_str());
        let mut line = format!(
            "{}{}: {}",
            behavior.id,
            if default { " (default)" } else { "" },
            behavior.description
        );
        if !behavior.speech_style.is_empty() {
            line.push_str(&format!("\n        speaks: {}", behavior.speech_style));
        }
        line
    };
    let trigger = |trigger: &PersonaTrigger| {
        let event = trigger
            .event
            .as_deref()
            .map_or_else(String::new, |event| format!(" or event {event}"));
        // A condition a model wrote often starts with "When" itself.
        let condition = trigger.condition.trim_start();
        let condition = match condition.get(..5) {
            Some(start) if start.eq_ignore_ascii_case("when ") => condition[5..].trim_start(),
            _ => condition,
        };
        format!(
            "{} -> {}: when {condition}{event}",
            trigger.id, trigger.behavior
        )
    };
    let blocks = [
        Some(format!("persona {} (owner)", persona.id())),
        text_block("display name", persona.display_name()),
        text_block("job", persona.job()),
        list_block("personality", persona.personality()),
        list_block("motivations", persona.private_motivations()),
        list_block("goals", persona.private_goals()),
        list_block("needs", persona.private_needs()),
        list_block("fears", persona.private_fears()),
        list_block("boundaries", persona.private_boundaries()),
        text_block("speech style", persona.speech_style()),
        text_block("backstory", persona.backstory()),
        text_block("private backstory", persona.private_backstory()),
        items_block("secrets", persona.private_secrets(), secret),
        items_block("behaviors", persona.private_behaviors(), behavior),
        items_block("triggers", persona.private_triggers(), trigger),
    ];
    blocks.into_iter().flatten().collect::<Vec<_>>().join("\n")
}
