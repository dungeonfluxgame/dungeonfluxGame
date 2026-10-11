//! Listener-safety data the generator derives from the rolled facts: the phrases that give the
//! private clue away, and who each speaking character is. The pack carries both so that an
//! in-character reply never has to guess them from prose.
use super::super::pack::SpeakerProfile;
use super::schema::{Antagonist, Destination, Secret};
use std::collections::BTreeMap;

/// The secret's own phrases, the destination's title and the contact's names, without
/// duplicates. Everything a player learns only from the private clue.
pub(super) fn secret_terms(
    secret: &Secret,
    destination: &Destination,
    context: &BTreeMap<String, String>,
) -> Vec<String> {
    let contact_first = context.get("contact.first").cloned().unwrap_or_default();
    let contact_surname = context
        .get("contact.full")
        .and_then(|full| full.split_whitespace().next_back())
        .map(str::to_owned)
        .unwrap_or_default();
    let mut terms: Vec<String> = Vec::new();
    for term in secret.terms.iter().cloned().chain([
        destination.title.clone(),
        contact_first,
        contact_surname,
    ]) {
        if !term.is_empty() && !terms.iter().any(|kept| kept.eq_ignore_ascii_case(&term)) {
            terms.push(term);
        }
    }
    terms
}

/// The patron, the contact and, when it speaks, the antagonist, under the names the scenes
/// show.
pub(super) fn speakers(
    context: &BTreeMap<String, String>,
    antagonist: &Antagonist,
    antagonist_name: &str,
) -> Vec<SpeakerProfile> {
    let field = |key: &str| context.get(key).cloned().unwrap_or_default();
    let mut profiles = vec![
        SpeakerProfile {
            name: field("patron.first"),
            role: format!("{} who asks the heroes for help", field("patron.desc")),
            voice: field("patron.voice"),
        },
        SpeakerProfile {
            name: field("contact.first"),
            role: format!(
                "{}, the heroes' contact at the end of the road",
                field("contact.desc")
            ),
            voice: field("contact.voice"),
        },
    ];
    if antagonist.speaks {
        profiles.push(SpeakerProfile {
            name: antagonist_name.to_owned(),
            role: format!(
                "a hostile {} fighting the heroes: taunts, threatens or parleys, never helps",
                antagonist.noun
            ),
            voice: antagonist.voice.clone(),
        });
    }
    profiles
}
