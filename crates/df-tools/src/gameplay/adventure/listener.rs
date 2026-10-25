//! Listener-safety data the generator derives from the rolled facts: the phrases that give the
//! private clue away, and who each speaking character is. The pack carries both so that an
//! in-character reply never has to guess them from prose.
use super::super::pack::{Age, CastRole, Gender, SceneCastMember, SpeakerProfile};
use super::schema::{Antagonist, Destination, Secret};
use df_render::art_contract;
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

/// The cut-out figure for a portrait: `portrait-` becomes `sprite-` in the asset stem. A role
/// token (the character has no reviewed photograph) is its own cut-out.
pub(super) fn sprite_of(portrait: &str) -> String {
    if art_contract::Token::parse(portrait).is_some() {
        return portrait.to_owned();
    }
    match portrait.rsplit_once("/portrait-") {
        Some((directory, stem)) => format!("{directory}/sprite-{stem}"),
        None => String::new(),
    }
}

/// One character on stage.
pub(super) fn cast_member(
    role: CastRole,
    id: &str,
    name: &str,
    portrait: &str,
    voice: (&str, &str, &str),
) -> SceneCastMember {
    let (voice, gender, age) = voice;
    SceneCastMember {
        gender: Gender::from_table(gender),
        age: Age::from_table(age),
        id: id.to_owned(),
        name: name.to_owned(),
        role,
        sprite: sprite_of(portrait),
        portrait: portrait.to_owned(),
        voice: voice.to_owned(),
        stat_block: None,
    }
}

/// The patron, the contact and, when it speaks, the antagonist, under the names the scenes
/// show.
pub(super) fn speakers(
    context: &BTreeMap<String, String>,
    antagonist: &Antagonist,
    antagonist_name: &str,
) -> Vec<SpeakerProfile> {
    let field = |key: &str| context.get(key).cloned().unwrap_or_default();
    let traits = |prefix: &str| {
        (
            Gender::from_table(&field(&format!("{prefix}.gender"))),
            Age::from_table(&field(&format!("{prefix}.age"))),
        )
    };
    let mut profiles = vec![
        SpeakerProfile {
            gender: traits("patron").0,
            age: traits("patron").1,
            name: field("patron.first"),
            role: format!("{} who asks the heroes for help", field("patron.desc")),
            voice: field("patron.voice"),
            identity_reply: None,
            withheld_subjects: Vec::new(),
        },
        SpeakerProfile {
            gender: traits("contact").0,
            age: traits("contact").1,
            name: field("contact.first"),
            role: format!(
                "{}, the heroes' contact at the end of the road",
                field("contact.desc")
            ),
            voice: field("contact.voice"),
            identity_reply: None,
            withheld_subjects: Vec::new(),
        },
    ];
    if antagonist.speaks {
        profiles.push(SpeakerProfile {
            gender: traits("ant").0,
            age: traits("ant").1,
            name: antagonist_name.to_owned(),
            role: format!(
                "a hostile {} fighting the heroes: taunts, threatens or parleys, never helps",
                antagonist.noun
            ),
            voice: antagonist.voice.clone(),
            identity_reply: None,
            withheld_subjects: Vec::new(),
        });
    }
    profiles
}
