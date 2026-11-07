//! Whole-word checks of a model's output against a character's private side.
//!
//! A persona's backstory and personality are what anyone can know of the character, and its
//! secrets, fears, goals, needs, motivations and private backstory are the character's own. The
//! model is told so, but it can still copy a private word into a public text, or into a line.
//! These checks look for that. They compare whole lowercase words of 4 letters or more, minus a
//! small stop-word list, and they only ever run on the model's output against a fixed list built
//! from the persona itself, never on what a player says (product pattern 8.1).
//!
//! They are whole-word checks: inflections ("guilds" for "guild") and paraphrase pass them, so
//! they reduce accidents and are not a guarantee. Two of them refuse nothing and report a
//! `leak_suspect` flag (a line, a generated persona); the third refuses a backfill that would put
//! a private word in a public field, which the caller turns into a typed error and no write.

use std::collections::HashSet;

use df_content::persona::{Persona, PersonaEdit, PersonaField};

/// The fewest letters a word needs to count as private detail; shorter words are too common.
const MINIMUM_TERM_LETTERS: usize = 4;

/// Words too common to show that a text copies a private one.
const STOP_WORDS: [&str; 40] = [
    "that", "this", "with", "from", "they", "have", "were", "been", "their", "what", "when",
    "will", "your", "about", "into", "than", "them", "then", "there", "which", "would", "could",
    "should", "never", "always", "only", "also", "just", "over", "after", "before", "where",
    "while", "because", "though", "being", "does", "some", "many", "most",
];

/// The lowercase words of `text` that could be private detail: whole words of
/// [`MINIMUM_TERM_LETTERS`] or more letters that are not stop words.
fn terms(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|word| word.chars().count() >= MINIMUM_TERM_LETTERS)
        .filter(|word| !STOP_WORDS.contains(&word.as_str()))
}

/// Every text of `persona` that anyone may know.
fn public_texts(persona: &Persona) -> impl Iterator<Item = &str> {
    [
        persona.display_name(),
        persona.job(),
        persona.speech_style(),
        persona.backstory(),
    ]
    .into_iter()
    .chain(persona.personality().iter().map(String::as_str))
}

/// Every text of `persona` that is the character's own.
fn private_texts(persona: &Persona) -> impl Iterator<Item = &str> {
    persona
        .private_secrets()
        .iter()
        .map(|secret| secret.text.as_str())
        .chain(persona.private_fears().iter().map(String::as_str))
        .chain(persona.private_goals().iter().map(String::as_str))
        .chain(persona.private_needs().iter().map(String::as_str))
        .chain(persona.private_motivations().iter().map(String::as_str))
        .chain(std::iter::once(persona.private_backstory()))
}

/// The terms of `persona`'s private side that its public side does not already use: what a new
/// public text could only have taken from the private side.
fn private_only_terms(persona: &Persona) -> HashSet<String> {
    let public: HashSet<String> = public_texts(persona).flat_map(terms).collect();
    private_texts(persona)
        .flat_map(terms)
        .filter(|term| !public.contains(term))
        .collect()
}

/// Whether an edit of `edits` adds a public text (an appended backstory or a personality entry)
/// that has a word of `persona`'s private side. A backfill that does is refused.
pub(super) fn copies_private_detail_into_public_text(
    persona: &Persona,
    edits: &[PersonaEdit],
) -> bool {
    let private = private_only_terms(persona);
    edits
        .iter()
        .filter_map(|edit| match edit {
            PersonaEdit::AppendText {
                field: PersonaField::Backstory,
                text,
            }
            | PersonaEdit::AddEntry {
                field: PersonaField::Personality,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .any(|text| terms(text).any(|term| private.contains(&term)))
}

/// Whether `line`, spoken by `persona`, has a word of its private side that neither the public
/// persona nor `given` (the question and the situation the line answers) uses. It blocks
/// nothing: the caller reports it.
pub(super) fn line_may_leak_private_detail(persona: &Persona, line: &str, given: &[&str]) -> bool {
    let private = private_only_terms(persona);
    let given: HashSet<String> = given.iter().copied().flat_map(terms).collect();
    terms(line).any(|term| private.contains(&term) && !given.contains(&term))
}

/// Whether the public texts of a generated `persona` that tell a story (its backstory and
/// personality) share a word with its private side. It blocks nothing: the caller reports it. A
/// shared word is only a suspicion, since a persona's public and private sides often belong
/// together.
pub(super) fn generated_persona_may_leak_private_detail(persona: &Persona) -> bool {
    let private: HashSet<String> = private_texts(persona).flat_map(terms).collect();
    std::iter::once(persona.backstory())
        .chain(persona.personality().iter().map(String::as_str))
        .flat_map(terms)
        .any(|term| private.contains(&term))
}
