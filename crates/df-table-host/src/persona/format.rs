//! The text each `persona` verb prints: keys and values, never a prompt. The owner view shows
//! everything a persona holds (the dev API is for the operator only).

use df_content::persona::backfill::Gap;
use df_content::persona::{
    Attitude, Persona, PersonaBehavior, PersonaId, PersonaSecret, PersonaState, PersonaTrigger,
    PublicPersona, Voice,
};

use crate::speech::format_voiced;

use super::request::{
    Answered, BackfillOutcome, BackfillRecord, Backfilled, CastOutcome, Covered, PersonaSummary,
    QuoteReport, SaidLine, Spoken,
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

/// The emotions above this level are shown in a reply.
const SHOWN_EMOTION_LEVEL: u8 = 20;

fn attitude_key(attitude: Attitude) -> &'static str {
    match attitude {
        Attitude::Friendly => "friendly",
        Attitude::Indifferent => "indifferent",
        Attitude::Hostile => "hostile",
    }
}

/// What the character feels after the line: the emotions above 20, the mood, the standing toward
/// each other character and the attention focus.
fn format_felt(state: &PersonaState) -> String {
    let mut felt: Vec<_> = state
        .emotions
        .iter()
        .filter(|level| level.level > SHOWN_EMOTION_LEVEL)
        .collect();
    felt.sort_by_key(|level| std::cmp::Reverse(level.level));
    let emotions = if felt.is_empty() {
        "calm".to_owned()
    } else {
        felt.iter()
            .map(|level| format!("{} {}", level.emotion, level.level))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut parts = vec![format!(
        "{emotions}; mood {} valence, {} arousal",
        state.mood.valence, state.mood.arousal
    )];
    for standing in &state.relationships {
        parts.push(format!(
            "toward {}: {} ({})",
            standing.toward,
            attitude_key(standing.attitude),
            standing.rapport
        ));
    }
    let attention = &state.attention;
    if let Some(focus) = &attention.focus {
        parts.push(match &attention.topic {
            Some(topic) => format!("focus {focus} on {topic}"),
            None => format!("focus {focus}"),
        });
    }
    parts.join("; ")
}

fn format_quote(said: &SaidLine, quote: &QuoteReport) -> String {
    let lot = format!("{} {}", quote.qty, quote.item);
    match (said.quoted_cp, quote.bounds) {
        (Some(quoted), Some(bounds))
            if u64::from(bounds.clamped_total_cp) != quote.proposed_total_cp =>
        {
            format!(
                concat!(
                    "  quoted: {} cp for {}; the line said {} cp, held to {} cp ",
                    "(floor {}, ask {})"
                ),
                quoted,
                lot,
                quote.proposed_total_cp,
                bounds.clamped_total_cp,
                bounds.floor_total_cp,
                bounds.ask_total_cp
            )
        }
        (Some(quoted), Some(bounds)) => format!(
            "  quoted: {quoted} cp for {lot} (inside floor {}, ask {})",
            bounds.floor_total_cp, bounds.ask_total_cp
        ),
        (Some(quoted), None) => {
            format!("  quoted: {quoted} cp for {lot} (not checked: no stall of theirs sells it)")
        }
        (None, _) => format!("  quoted: {lot}"),
    }
}

/// The lines about a spoken line; `after_line` is what follows "time: line Ns" (the backfill).
fn spoken_lines(spoken: &Spoken, after_line: &str) -> Vec<String> {
    let said = &spoken.said;
    let mut lines = vec![format!(
        "ok (revision {}) {} says: {}",
        spoken.revision, spoken.character, said.line
    )];
    if let Some(voice) = &spoken.voice {
        lines.push(format_voiced(voice));
    }
    if let Some(quote) = &said.quote {
        lines.push(format_quote(said, quote));
    }
    if said.leak_suspect {
        lines.push(
            "  leak suspect: the model judged the line gives away the private side".to_owned(),
        );
    }
    if let Some(behavior) = &said.behavior {
        lines.push(format!("  behavior: {behavior}"));
    }
    if let Some(over) = &said.behavior_override {
        lines.push(format!(
            "  behavior override: trigger {} fired, so the line took {}, not {}",
            over.trigger,
            over.took,
            over.model.as_deref().unwrap_or("none")
        ));
    }
    if !said.fired_triggers.is_empty() {
        lines.push(format!(
            "  fired triggers: {}",
            said.fired_triggers.join(", ")
        ));
    }
    if !said.fired_emotional_triggers.is_empty() {
        lines.push(format!(
            "  fired emotional triggers: {} (by state: {})",
            said.fired_emotional_triggers.join(", "),
            if said.state_triggered.is_empty() {
                "none".to_owned()
            } else {
                said.state_triggered.join(", ")
            }
        ));
    }
    if !said.revealed_secrets.is_empty() {
        lines.push(format!(
            "  revealed secrets: {}",
            said.revealed_secrets.join(", ")
        ));
    }
    if said.repeat_retried {
        lines.push(format!(
            "  repeat: the first line reused recent, worded again{}",
            if said.repeats_recent {
                "; the second still does"
            } else {
                ""
            }
        ));
    }
    if let Some(tone) = &said.tone {
        lines.push(format!("  tone: {tone}"));
    }
    lines.push(format!("  feels: {}", format_felt(&said.state)));
    if !said.dropped.is_empty() {
        lines.push(format!(
            "  dropped from the answer: {}",
            said.dropped.join(", ")
        ));
    }
    lines.push(format!(
        "  time: line {:.1}s{after_line}",
        said.model_ms as f64 / 1000.0
    ));
    lines
}

pub(super) fn format_spoken(spoken: &Spoken) -> String {
    spoken_lines(spoken, "").join("\n")
}

/// The gaps as `field: topic` lines.
fn gap_lines(gaps: &[Gap]) -> Vec<String> {
    gaps.iter()
        .map(|gap| format!("    - {}: {}", gap.field.key(), gap.topic))
        .collect()
}

pub(super) fn format_answered(answered: &Answered) -> String {
    let after_line = match &answered.backfill {
        BackfillOutcome::Running { .. } => ", backfill started",
        BackfillOutcome::Backfilled { .. } if !answered.spoken.said.gaps.is_empty() => {
            ", backfill waited for"
        }
        _ => "",
    };
    let mut lines = spoken_lines(&answered.spoken, after_line);
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
        BackfillOutcome::Running { record } => {
            format!("  backfill: running (record {record} when done)")
        }
        BackfillOutcome::Busy => {
            "  backfill: a backfill of this persona is already running; these gaps wait".to_owned()
        }
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

/// The voice as `  voice:` over its parts that say something.
fn voice_block(voice: &Voice) -> Option<String> {
    let text =
        |label: &str, text: &str| (!text.trim().is_empty()).then(|| format!("{label}: {text}"));
    let list = |label: &str, entries: &[String]| {
        (!entries.is_empty()).then(|| format!("{label}: {}", entries.join(" | ")))
    };
    let parts: Vec<String> = [
        text("rhythm", &voice.rhythm),
        list("favored", &voice.diction.favored),
        list("never", &voice.diction.never),
        list("tics", &voice.tics),
        text("address", &voice.address),
        list("idioms", &voice.idioms),
        text("deflection", &voice.deflection),
        list("samples", &voice.samples),
    ]
    .into_iter()
    .flatten()
    .map(|part| format!("    {part}"))
    .collect();
    (!parts.is_empty()).then(|| format!("  voice:\n{}", parts.join("\n")))
}

/// What `persona voice <id> generate` wrote.
pub(super) fn format_voice_generated(persona: &Persona) -> String {
    let packs = if persona.voice_packs().is_empty() {
        "none".to_owned()
    } else {
        persona.voice_packs().join(", ")
    };
    format!(
        "ok voice written for {} ({} samples, {} expertise entries, packs: {packs})",
        persona.id(),
        persona.voice().samples.len(),
        persona.expertise().len()
    )
}

pub(super) fn format_public(persona: &PublicPersona) -> String {
    let blocks = [
        Some(format!("persona {} (public)", persona.id)),
        text_block("display name", &persona.display_name),
        text_block("job", &persona.job),
        list_block("personality", &persona.personality),
        text_block("speech style", &persona.speech_style),
        voice_block(&persona.voice),
        list_block("expertise", &persona.expertise),
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
        voice_block(persona.voice()),
        list_block("voice packs", persona.voice_packs()),
        list_block("expertise", persona.expertise()),
        text_block("backstory", persona.backstory()),
        text_block("private backstory", persona.private_backstory()),
        items_block("secrets", persona.private_secrets(), secret),
        items_block("behaviors", persona.private_behaviors(), behavior),
        items_block("triggers", persona.private_triggers(), trigger),
    ];
    blocks.into_iter().flatten().collect::<Vec<_>>().join("\n")
}
