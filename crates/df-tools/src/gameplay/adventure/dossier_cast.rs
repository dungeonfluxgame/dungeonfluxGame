//! Adapter from a generated adventure to the NPC dossiers of its speaking cast; governing design:
//! adventure-generator §19.11. Pure: no I/O and no draw from the adventure's own dice stream
//! (dossiers roll on their own per-NPC streams), so attaching dossiers never changes what a seed
//! generates.
use super::{
    GenerateError, GeneratedAdventure, RollStatus, Tables,
    dossier::{
        AntagonistFact, AuthoredLine, CastMember, Dossier, DossierFacts, Gender, Group, NpcRole,
        StakeFact, roll_dossiers,
    },
};
use std::collections::BTreeMap;

/// What the party may already hear, and any secret the generator's own facts do not carry.
#[derive(Clone, Debug, Default)]
pub(super) struct PublicStory {
    /// Name or alias by which the party may already hear the villain spoken of.
    pub(super) villain: Option<String>,
    pub(super) faction: Option<String>,
    /// Extra phrases no public line may contain (a campaign's private lead, the villain's name).
    pub(super) extra_secret_terms: Vec<String>,
}

fn fact<'a>(facts: &'a BTreeMap<String, String>, key: &str) -> Result<&'a String, GenerateError> {
    facts
        .get(key)
        .ok_or_else(|| GenerateError::Pattern(format!("dossier fact {key}")))
}

fn gender_of(facts: &BTreeMap<String, String>, prefix: &str) -> Result<Gender, GenerateError> {
    Ok(if fact(facts, &format!("{prefix}.sub"))? == "she" {
        Gender::She
    } else {
        Gender::He
    })
}

fn group_of(class: &str) -> Group {
    match class {
        "person" => Group::Person,
        "goblinoid" => Group::Goblinoid,
        "undead" => Group::Undead,
        _ => Group::Beast,
    }
}

/// The statements each named speaker makes in the authored scenes, in scene order. A question is
/// a request the scene already made, not something the speaker knows, so it is left out.
pub(super) fn authored_lines(scenes: &[super::super::pack::Scene]) -> Vec<AuthoredLine> {
    scenes
        .iter()
        .filter(|scene| !scene.speaker.trim().is_empty())
        .flat_map(|scene| {
            statements(&scene.dialogue)
                .into_iter()
                .map(move |text| AuthoredLine {
                    speaker: scene.speaker.clone(),
                    text,
                })
        })
        .collect()
}

/// Sentences of `text` that end in `.` or `!`, each trimmed. A sentence ends at a terminal mark
/// followed by a space or the end of the text.
fn statements(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut start = 0;
    let mut characters = text.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        let ends_here = matches!(character, '.' | '!' | '?')
            && characters
                .peek()
                .is_none_or(|(_, next)| next.is_whitespace());
        if !ends_here {
            continue;
        }
        let end = index + character.len_utf8();
        let sentence = text[start..end].trim();
        if !sentence.is_empty() && !sentence.ends_with('?') {
            found.push(sentence.to_owned());
        }
        start = end;
    }
    let rest = text[start..].trim();
    if !rest.is_empty() && !rest.ends_with('?') {
        found.push(rest.to_owned());
    }
    found
}

/// The speaking cast and shared facts of an adventure: the patron and contact, plus the
/// antagonist only when it speaks. Ids are `"<role>:<archetype id>"`.
fn dossier_facts(
    tables: &Tables,
    adventure: &GeneratedAdventure,
    story: &PublicStory,
) -> Result<DossierFacts, GenerateError> {
    let facts = &adventure.facts;
    let mut cast = Vec::new();
    for (prefix, role) in [("patron", NpcRole::Patron), ("contact", NpcRole::Contact)] {
        let archetype = fact(facts, &format!("{prefix}.id"))?;
        cast.push(CastMember {
            id: format!("{}:{archetype}", role.label()),
            name: fact(facts, &format!("{prefix}.first"))?.clone(),
            role,
            role_label: fact(facts, &format!("{prefix}.role"))?.clone(),
            archetype: archetype.clone(),
            gender: gender_of(facts, prefix)?,
            group: Group::Person,
            mannerism: Some(fact(facts, &format!("{prefix}.mannerism"))?.clone()),
            delivery: Some(fact(facts, &format!("{prefix}.delivery"))?.clone()),
            voice: Some(fact(facts, &format!("{prefix}.voice"))?.clone()),
        });
    }
    let antagonist_id = fact(facts, "ant.id")?;
    let creature = tables
        .antagonists
        .entries
        .iter()
        .find(|entry| &entry.id == antagonist_id)
        .ok_or_else(|| GenerateError::Pattern("dossier antagonist".into()))?;
    let group = group_of(&creature.klass);
    if creature.speaks {
        let short = fact(facts, "ant.short")?;
        cast.push(CastMember {
            id: format!("{}:{}", NpcRole::Antagonist.label(), creature.id),
            // "Snag the Goblin" is called Snag; "Wystan Oakes" keeps both names.
            name: short.split(" the ").next().unwrap_or(short).to_owned(),
            role: NpcRole::Antagonist,
            role_label: fact(facts, "ant.the")?
                .trim_start_matches("the ")
                .to_owned(),
            archetype: creature.id.clone(),
            gender: gender_of(facts, "ant")?,
            group,
            mannerism: None,
            delivery: facts.get("ant.delivery").cloned(),
            voice: Some(fact(facts, "ant.voice")?.clone()),
        });
    }
    // The secret's id is a recorded roll, so it survives a campaign overlay that replaces the
    // rendered secret facts.
    let secret_id = adventure
        .rolls
        .iter()
        .find(|roll| roll.table == "secret" && roll.status != RollStatus::Rejected)
        .map(|roll| roll.entry_id.clone())
        .ok_or_else(|| GenerateError::Pattern("dossier secret".into()))?;
    let secret_terms = ["secret.fact", "secret.confirm"]
        .into_iter()
        .filter_map(|key| facts.get(key))
        .chain(&story.extra_secret_terms)
        .filter(|term| !term.trim().is_empty())
        .cloned()
        .collect();
    Ok(DossierFacts {
        authored: authored_lines(&adventure.pack.scenes),
        cast,
        stake: StakeFact {
            short: fact(facts, "stake.short")?.clone(),
            full: fact(facts, "stake.full")?.clone(),
        },
        location_id: fact(facts, "loc.id")?.clone(),
        location: fact(facts, "loc.name")?.clone(),
        destination: fact(facts, "dest.name")?.clone(),
        antagonist: AntagonistFact {
            short: fact(facts, "ant.short")?.clone(),
            the: fact(facts, "ant.the")?.clone(),
            group,
        },
        villain: story.villain.clone(),
        faction: story.faction.clone(),
        secret_id,
        secret_terms,
    })
}

/// Rolls the dossiers of an adventure's speaking cast from the adventure seed.
pub(super) fn roll_for(
    tables: &Tables,
    adventure: &GeneratedAdventure,
    story: &PublicStory,
) -> Result<Vec<Dossier>, GenerateError> {
    let facts = dossier_facts(tables, adventure, story)?;
    roll_dossiers(
        &tables.for_rating(adventure.rating).dossiers,
        &facts,
        adventure.seed,
    )
    .map_err(GenerateError::Dossier)
}
