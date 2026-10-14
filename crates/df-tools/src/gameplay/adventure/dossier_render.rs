//! Plain-text DM sheet for one dossier; governing design: adventure-generator §19.
#![allow(
    dead_code,
    reason = "consumed by the adapter and the reply task, which land after this crate-internal contract"
)]
use super::dossier::{Dossier, ForbiddenScope, Knowledge, Reaction, Topic};
use std::fmt::Write;

fn knowledge_lines(out: &mut String, items: &[Knowledge]) {
    for item in items {
        let _ = writeln!(out, "  - [{}] {}", item.topic.label(), item.text);
    }
}
fn reaction_line(out: &mut String, label: &str, reaction: &Reaction) {
    let _ = writeln!(out, "  {label}: {}", reaction.text);
    let _ = writeln!(out, "      says: \"{}\"", reaction.answer);
}

/// A readable DM-style sheet: how to play this person, and why they are in this story.
pub(crate) fn render_dossier(dossier: &Dossier) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "=== {} ({}, {}) ===",
        dossier.name,
        dossier.role.label(),
        dossier.role_label
    );
    let _ = writeln!(
        out,
        "Starts {} toward the party.  [{}]",
        dossier.starting_attitude.label(),
        dossier.npc_id
    );
    let given = &dossier.given;
    let _ = writeln!(
        out,
        "Given: voice {}; manner {}; delivery {}.",
        given.voice.as_deref().unwrap_or("-"),
        given.mannerism.as_deref().unwrap_or("-"),
        given.delivery.as_deref().unwrap_or("-")
    );
    if let Some(note) = &given.note {
        let _ = writeln!(out, "Note: {note}");
    }
    let style = &dossier.speech_style;
    let _ = writeln!(
        out,
        "Speech: {:?} sentences; {}; tic: {} (opens \"{}\", closes \"{}\").",
        style.length, style.vocabulary, style.tic, style.opener, style.closer
    );
    let tie = &dossier.story_tie;
    let _ = writeln!(
        out,
        "\nWHY THIS PERSON IS IN THIS STORY ({}{}):",
        match tie.binds {
            super::dossier::Binding::Cast => "bound to another cast member",
            super::dossier::Binding::Stake => "bound to the stake",
            super::dossier::Binding::Antagonist => "bound to the antagonist",
            super::dossier::Binding::Villain => "bound to the villain",
            super::dossier::Binding::Faction => "bound to a faction",
            super::dossier::Binding::Location => "bound to the location",
            super::dossier::Binding::Destination => "bound to the destination",
        },
        if tie.public { ", public" } else { ", private" }
    );
    let _ = writeln!(out, "  {}", tie.text);
    let _ = writeln!(
        out,
        "  In their own words{}: \"{}\"",
        if tie.public {
            ""
        } else {
            " (only once trusted)"
        },
        tie.speak
    );
    if let Some(motive) = &dossier.motive {
        let _ = writeln!(out, "  Motive: {}", motive.text);
    }
    if let Some(stand_down) = &dossier.stand_down {
        let _ = writeln!(out, "  Backs off or bargains when: {}", stand_down.text);
    }
    let _ = writeln!(out, "\nBACKGROUND:");
    for (label, item) in [
        ("Origin", &dossier.origin),
        ("Past trade", &dossier.past_trade),
        ("Formative event", &dossier.formative_event),
        ("Talent", &dossier.talent),
    ] {
        let _ = writeln!(out, "  {label}: {}", item.text);
    }
    let _ = writeln!(out, "\nPERSONALITY:");
    for (label, item) in [
        ("Ideal", &dossier.ideal),
        ("Bond", &dossier.bond),
        ("Flaw", &dossier.flaw),
        ("Wants now", &dossier.want),
        ("Fear", &dossier.fear),
    ] {
        let _ = writeln!(out, "  {label}: {}", item.text);
    }
    let _ = writeln!(out, "\nKNOWLEDGE");
    let _ = writeln!(out, " Says freely:");
    knowledge_lines(&mut out, &dossier.knowledge.says_freely);
    let _ = writeln!(out, " Says only when trusted (attitude must allow it):");
    knowledge_lines(&mut out, &dossier.knowledge.says_when_trusted);
    let _ = writeln!(out, " Never says:");
    for item in &dossier.knowledge.never_says {
        let scope = match item.scope {
            ForbiddenScope::Everywhere => "never, to anyone",
            ForbiddenScope::PublicOnly => "never freely",
        };
        let _ = writeln!(out, "  - ({scope}) {}", item.text);
    }
    let profile = dossier.emotional_profile();
    let _ = writeln!(
        out,
        "\nEMOTIONAL PROFILE (story colour, not a mechanical benefit):"
    );
    let _ = writeln!(out, "  Orientation: {}", profile.orientation);
    let _ = writeln!(
        out,
        "  Forgiveness: {}",
        match profile.forgiveness {
            super::dossier::Forgiveness::Forgiving => "forgiving",
            super::dossier::Forgiveness::Unforgiving => "unforgiving",
        }
    );
    let _ = writeln!(
        out,
        "  Patience: {}",
        match profile.patience {
            super::dossier::Patience::Low => "low",
            super::dossier::Patience::Middling => "middling",
            super::dossier::Patience::High => "high",
        }
    );
    for feeling in &profile.feelings {
        let _ = writeln!(
            out,
            "  {:?}: {} (because: {})",
            feeling.kind, feeling.text, feeling.because
        );
    }
    for tell in &profile.tells {
        let _ = writeln!(out, "  Tell: {tell}");
    }
    let _ = writeln!(out, "  Legacy approach reactions:");
    reaction_line(&mut out, "Flattery", &profile.legacy_reactions.flattery);
    reaction_line(&mut out, "Flirting", &profile.legacy_reactions.flirting);
    reaction_line(&mut out, "Threats", &profile.legacy_reactions.threats);
    reaction_line(&mut out, "Lies", &profile.legacy_reactions.lies);
    reaction_line(&mut out, "Bribes", &profile.legacy_reactions.bribes);
    let _ = writeln!(
        out,
        "\nPREPARED ANSWERS (fallback when no text model is on):"
    );
    for topic in Topic::ALL {
        if let Some(answer) = dossier.prepared_answers.get(&topic) {
            let _ = writeln!(out, "  {}: \"{answer}\"", topic.label());
        }
    }
    out
}
