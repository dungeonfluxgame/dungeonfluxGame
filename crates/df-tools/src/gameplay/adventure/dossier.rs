//! NPC backstory dossiers; governing design: adventure-generator §19.
//!
//! A dossier is a pure function of `(adventure seed, NPC id, facts, tables)`. Every NPC rolls on an
//! independent SplitMix64 stream derived from the adventure seed and the NPC id, so adding the
//! dossier layer never changes the chapter generator's own rolls. The module performs no I/O and
//! no model call. Nothing in a dossier is a game rule: attitude, knowledge and temperament only
//! guide how the NPC talks.
#![allow(
    dead_code,
    reason = "consumed by the adapter and the reply task, which land after this crate-internal contract"
)]
use super::{
    Roll, RollStatus,
    dossier_tables::{DossierTables, Faced, Profile, StyleEntry, Tie, slots_of},
    render,
    rng::Dice,
    schema::Table,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

/// Domain separation so a dossier stream never equals the adventure's own stream.
const STREAM_TAG: u64 = 0x4e50_4344_4f53_5331;
const MAX_ATTEMPTS: usize = 64;
const MAX_CAST: usize = 20;
const MAX_SENTENCES: usize = 3;
const DICE: [u8; 6] = [4, 6, 8, 10, 12, 20];

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NpcRole {
    Patron,
    Contact,
    Antagonist,
    Villain,
    Lieutenant,
}
impl NpcRole {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Patron => "patron",
            Self::Contact => "contact",
            Self::Antagonist => "antagonist",
            Self::Villain => "villain",
            Self::Lieutenant => "lieutenant",
        }
    }
    /// Roles that carry a stated motive and a stand-down condition.
    pub(crate) fn opposes_party(self) -> bool {
        matches!(self, Self::Antagonist | Self::Villain | Self::Lieutenant)
    }
}
/// Creature group of a speaker (the NPC) or of the chapter antagonist (what is talked about).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Group {
    Person,
    Goblinoid,
    Beast,
    Undead,
}
impl Group {
    pub(crate) fn speaks(self) -> bool {
        matches!(self, Self::Person | Self::Goblinoid)
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Gender {
    He,
    She,
}
impl Gender {
    fn pronouns(self) -> [&'static str; 3] {
        match self {
            Self::He => ["he", "him", "his"],
            Self::She => ["she", "her", "her"],
        }
    }
}
/// Standard 2024 attitude scale. The dossier only records where the NPC starts; the server owns
/// and saves the live value.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Attitude {
    Friendly,
    Indifferent,
    Hostile,
}
impl Attitude {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Friendly => "friendly",
            Self::Indifferent => "indifferent",
            Self::Hostile => "hostile",
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Length {
    Short,
    Medium,
    Long,
}
/// The fixed topics a player is most likely to raise; each has a prepared in-character answer.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Topic {
    WhoAreYou,
    YourPast,
    WhyThisMatters,
    TheAntagonist,
    ThePlace,
    ComplimentOrFlirt,
    Threat,
    Bribe,
    WhatAreYouHiding,
}
impl Topic {
    pub(crate) const ALL: [Self; 9] = [
        Self::WhoAreYou,
        Self::YourPast,
        Self::WhyThisMatters,
        Self::TheAntagonist,
        Self::ThePlace,
        Self::ComplimentOrFlirt,
        Self::Threat,
        Self::Bribe,
        Self::WhatAreYouHiding,
    ];
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::WhoAreYou => "who are you",
            Self::YourPast => "your past",
            Self::WhyThisMatters => "why this matters to you",
            Self::TheAntagonist => "the antagonist",
            Self::ThePlace => "the place",
            Self::ComplimentOrFlirt => "a compliment or flirt",
            Self::Threat => "a threat",
            Self::Bribe => "a bribe",
            Self::WhatAreYouHiding => "what are you hiding",
        }
    }
    fn ordinal(self) -> usize {
        Self::ALL
            .iter()
            .position(|topic| *topic == self)
            .unwrap_or(0)
    }
}
/// Which adventure fact a story tie binds the NPC to.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Binding {
    Stake,
    Antagonist,
    Villain,
    Faction,
    Location,
    Destination,
    Cast,
}
impl Binding {
    /// Template slots whose value names the bound fact.
    pub(super) fn slots(self) -> &'static [&'static str] {
        match self {
            Self::Stake => &["stake", "stake_full"],
            Self::Antagonist => &["antagonist", "antagonist_the"],
            Self::Villain => &["villain"],
            Self::Faction => &["faction"],
            Self::Location => &["location"],
            Self::Destination => &["destination"],
            Self::Cast => &["other"],
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ForbiddenScope {
    /// Never spoken, in any tier.
    Everywhere,
    /// Never spoken in a public tier; the trust-gated tier may still touch it.
    PublicOnly,
}

/// One cast member to flesh out. `id` must be stable and unique within the adventure; the
/// adapter should use `"<role>:<archetype id>"`.
#[derive(Clone, Debug)]
pub(crate) struct CastMember {
    pub(crate) id: String,
    /// Name the party hears (first name).
    pub(crate) name: String,
    pub(crate) role: NpcRole,
    /// Article-free role noun such as "bell ringer" or "thief".
    pub(crate) role_label: String,
    /// Archetype id (for example "bell-ringer"); used only to avoid a past trade equal to the job.
    pub(crate) archetype: String,
    pub(crate) gender: Gender,
    /// Must be `Person` or `Goblinoid`; silent creatures have no dossier.
    pub(crate) group: Group,
    /// Existing mannerism, speech direction and voice tag, exactly as the chapter generator
    /// produced them; the dossier never contradicts them.
    pub(crate) mannerism: Option<String>,
    pub(crate) delivery: Option<String>,
    pub(crate) voice: Option<String>,
}
#[derive(Clone, Debug)]
pub(crate) struct StakeFact {
    /// For example "the brass key".
    pub(crate) short: String,
    /// For example "a brass key wrapped in blue cloth".
    pub(crate) full: String,
}
#[derive(Clone, Debug)]
pub(crate) struct AntagonistFact {
    /// Display name, for example "Snag the Goblin" or "the Scarred Giant Rat".
    pub(crate) short: String,
    /// For example "the goblin".
    pub(crate) the: String,
    pub(crate) group: Group,
}
/// Exactly what the ties need; the adapter builds it from the generated adventure.
#[derive(Clone, Debug)]
pub(crate) struct DossierFacts {
    pub(crate) cast: Vec<CastMember>,
    pub(crate) stake: StakeFact,
    /// Location table id (for example "lamplit-market"); selects place colour.
    pub(crate) location_id: String,
    /// Location name with article (for example "the Lamplit Market").
    pub(crate) location: String,
    pub(crate) destination: String,
    pub(crate) antagonist: AntagonistFact,
    /// Name or alias by which the party may already hear the villain spoken of; `None` while
    /// even that must stay hidden.
    pub(crate) villain: Option<String>,
    pub(crate) faction: Option<String>,
    pub(crate) secret_id: String,
    /// Rendered secret fact and confirmation text; the dossier tables add keyword phrases.
    pub(crate) secret_terms: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Trait {
    pub(crate) id: String,
    pub(crate) text: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct GivenTraits {
    pub(crate) mannerism: Option<String>,
    pub(crate) delivery: Option<String>,
    pub(crate) voice: Option<String>,
    /// Tone tags read from the given traits; temperament and style never clash with them.
    pub(crate) tags: Vec<String>,
    /// Present when the given traits already pull two ways; tells the DM how to play it.
    pub(crate) note: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoryTie {
    pub(crate) id: String,
    pub(crate) binds: Binding,
    /// The adventure fact's text exactly as it appears in `text`.
    pub(crate) bound_fact: String,
    /// Set when the tie binds another cast member.
    pub(crate) bound_cast: Option<String>,
    /// A public tie may be said freely; a private one waits for trust.
    pub(crate) public: bool,
    /// DM-facing statement.
    pub(crate) text: String,
    /// What the NPC says in the first person.
    pub(crate) speak: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Knowledge {
    pub(crate) topic: Topic,
    /// Which line this is (also the prepared-answer slot name, for example "place_line").
    pub(crate) source: String,
    pub(crate) text: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Forbidden {
    pub(crate) source: String,
    pub(crate) text: String,
    /// Whole-word, case-insensitive phrases that must not be spoken.
    pub(crate) terms: Vec<String>,
    pub(crate) scope: ForbiddenScope,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct KnowledgeTiers {
    pub(crate) says_freely: Vec<Knowledge>,
    pub(crate) says_when_trusted: Vec<Knowledge>,
    pub(crate) never_says: Vec<Forbidden>,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reaction {
    pub(crate) id: String,
    /// DM-facing description.
    pub(crate) text: String,
    /// First-person line, family-suitable, never a mechanical benefit.
    pub(crate) answer: String,
    pub(crate) tags: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Temperament {
    pub(crate) flattery: Reaction,
    pub(crate) flirting: Reaction,
    pub(crate) threats: Reaction,
    pub(crate) lies: Reaction,
    pub(crate) bribes: Reaction,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpeechStyle {
    pub(crate) id: String,
    pub(crate) length: Length,
    pub(crate) vocabulary: String,
    pub(crate) tic: String,
    pub(crate) opener: String,
    pub(crate) closer: String,
    pub(crate) tags: Vec<String>,
}
/// A complete backstory, persisted with the adventure record.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Dossier {
    pub(crate) npc_id: String,
    pub(crate) name: String,
    pub(crate) role: NpcRole,
    pub(crate) role_label: String,
    pub(crate) tables_revision: String,
    pub(crate) given: GivenTraits,
    pub(crate) origin: Trait,
    pub(crate) past_trade: Trait,
    pub(crate) formative_event: Trait,
    pub(crate) ideal: Trait,
    pub(crate) bond: Trait,
    pub(crate) flaw: Trait,
    pub(crate) want: Trait,
    pub(crate) fear: Trait,
    pub(crate) talent: Trait,
    pub(crate) story_tie: StoryTie,
    /// Antagonist-side roles only: why they are doing this.
    pub(crate) motive: Option<Trait>,
    /// Antagonist-side roles only: what would make them back off or bargain.
    pub(crate) stand_down: Option<Trait>,
    pub(crate) knowledge: KnowledgeTiers,
    pub(crate) temperament: Temperament,
    pub(crate) starting_attitude: Attitude,
    pub(crate) speech_style: SpeechStyle,
    pub(crate) prepared_answers: BTreeMap<Topic, String>,
    pub(crate) rolls: Vec<Roll>,
}

/// A forbidden phrase found in text it must not appear in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Leak {
    pub(crate) field: String,
    pub(crate) term: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DossierError {
    EmptyCast,
    TooManyCast(usize),
    DuplicateNpc(String),
    /// A cast member that cannot speak (beast or undead) was given.
    SilentSpeaker(String),
    EmptyFact(&'static str),
    UnknownSecret(String),
    NoCompatibleEntry {
        npc_id: String,
        table: String,
    },
    Template {
        npc_id: String,
        table: String,
    },
    Leak {
        npc_id: String,
        field: String,
        term: String,
    },
}
impl fmt::Display for DossierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCast => f.write_str("dossier cast is empty"),
            Self::TooManyCast(count) => write!(f, "dossier cast of {count} exceeds {MAX_CAST}"),
            Self::DuplicateNpc(id) => write!(f, "dossier npc id repeated: {id}"),
            Self::SilentSpeaker(id) => write!(f, "dossier npc cannot speak: {id}"),
            Self::EmptyFact(name) => write!(f, "dossier fact is empty: {name}"),
            Self::UnknownSecret(id) => write!(f, "dossier tables lack secret keywords: {id}"),
            Self::NoCompatibleEntry { npc_id, table } => {
                write!(f, "dossier {npc_id}: no compatible entry in {table}")
            }
            Self::Template { npc_id, table } => {
                write!(f, "dossier {npc_id}: template in {table} cannot be filled")
            }
            Self::Leak {
                npc_id,
                field,
                term,
            } => write!(
                f,
                "dossier {npc_id}: {field} mentions forbidden term {term}"
            ),
        }
    }
}
impl std::error::Error for DossierError {}

/// Lower-cased words; apostrophes and hyphens stay inside a word.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}
/// Case-insensitive whole-word match of a (possibly multi-word) phrase.
pub(crate) fn mentions_term(text: &str, term: &str) -> bool {
    let wanted = words(term);
    if wanted.is_empty() {
        return false;
    }
    words(text)
        .windows(wanted.len())
        .any(|window| window == wanted.as_slice())
}
/// Runs of `.`, `!` or `?` end a sentence; unterminated trailing text counts as one.
pub(crate) fn sentence_count(text: &str) -> usize {
    let mut count = 0;
    let mut in_run = false;
    for c in text.trim_end().chars() {
        let terminal = matches!(c, '.' | '!' | '?');
        if terminal && !in_run {
            count += 1;
        }
        in_run = terminal;
    }
    if !text.trim_end().ends_with(['.', '!', '?']) && !text.trim().is_empty() {
        count += 1;
    }
    count
}
fn with_article(label: &str) -> String {
    let vowel = label
        .chars()
        .next()
        .is_some_and(|c| matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u'));
    format!("{} {label}", if vowel { "an" } else { "a" })
}
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}
fn admits<T: PartialEq>(allowed: &[T], value: T) -> bool {
    allowed.is_empty() || allowed.contains(&value)
}
fn clashing(pairs: &[[String; 2]], left: &BTreeSet<String>, right: &BTreeSet<String>) -> bool {
    pairs.iter().any(|[a, b]| {
        (left.contains(a) && right.contains(b)) || (left.contains(b) && right.contains(a))
    })
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Audience {
    /// Said to anyone: no secret term, no shame term.
    Public,
    /// Said once trusted: no shame term, secret terms allowed.
    Trusted,
    /// Only the DM reads it.
    DmOnly,
}

/// The independent per-NPC dice stream with its recorded rolls.
struct Roller<'a> {
    npc_id: &'a str,
    dice: Dice,
    rolls: Vec<Roll>,
}
impl<'a> Roller<'a> {
    fn new(seed: u64, npc_id: &'a str) -> Self {
        Self {
            npc_id,
            dice: Dice::new(seed ^ STREAM_TAG ^ fnv1a(npc_id)),
            rolls: Vec::new(),
        }
    }
    fn record(&mut self, table: &str, die: u8, natural: u8, entry_id: &str, status: RollStatus) {
        self.rolls.push(Roll {
            n: self.rolls.len() + 1,
            table: format!("dossier.{table}"),
            die,
            natural,
            entry_id: entry_id.to_owned(),
            status,
        });
    }
    /// Roll on a table; an entry that does not fit is recorded as rejected and the same die is
    /// rolled again, exactly as the chapter generator does. After 64 rejections the first
    /// fitting entry in table order is taken and recorded as a fallback.
    fn pick<'t, T: Faced>(
        &mut self,
        name: &str,
        table: &'t Table<T>,
        fits: impl Fn(&T) -> bool,
    ) -> Result<&'t T, DossierError> {
        let npc_id = self.npc_id;
        let missing = || DossierError::NoCompatibleEntry {
            npc_id: npc_id.to_owned(),
            table: name.to_owned(),
        };
        for _ in 0..MAX_ATTEMPTS {
            let natural = self.dice.roll(table.die);
            let entry = table
                .entries
                .iter()
                .find(|entry| {
                    let [first, last] = entry.faces();
                    (first..=last).contains(&natural)
                })
                .ok_or_else(missing)?;
            let accepted = fits(entry);
            self.record(
                name,
                table.die,
                natural,
                entry.entry_id(),
                if accepted {
                    RollStatus::Ok
                } else {
                    RollStatus::Rejected
                },
            );
            if accepted {
                return Ok(entry);
            }
        }
        let entry = table
            .entries
            .iter()
            .find(|entry| fits(entry))
            .ok_or_else(missing)?;
        self.record(name, table.die, 0, entry.entry_id(), RollStatus::Fallback);
        Ok(entry)
    }
    /// Roll on a table of profiles. The DM text is never restricted; the line the NPC would
    /// speak (confided, spoken, or the plain text) must be allowed for `audience`.
    fn profile<'t>(
        &mut self,
        context: &Context<'_>,
        name: &str,
        table: &'t Table<Profile>,
        audience: Audience,
    ) -> Result<&'t Profile, DossierError> {
        self.pick(name, table, |entry| match spoken_template(entry) {
            Some(line) => context.profile_fits(
                entry,
                &[
                    (Some(&entry.text), Audience::DmOnly),
                    (Some(line), audience),
                ],
            ),
            None => context.profile_fits(entry, &[(Some(&entry.text), audience)]),
        })
    }
    /// Choose one of `count` items with the smallest standard die that holds them.
    fn pick_index(&mut self, name: &str, count: usize) -> Result<usize, DossierError> {
        let die = DICE
            .iter()
            .copied()
            .find(|die| usize::from(*die) >= count)
            .ok_or(DossierError::TooManyCast(count))?;
        loop {
            let natural = self.dice.roll(die);
            let accepted = usize::from(natural) <= count;
            self.record(
                name,
                die,
                natural,
                &natural.to_string(),
                if accepted {
                    RollStatus::Ok
                } else {
                    RollStatus::Rejected
                },
            );
            if accepted {
                return Ok(usize::from(natural) - 1);
            }
        }
    }
}

/// Everything fixed about one NPC while its dice are rolled.
#[derive(Clone)]
struct Context<'a> {
    tables: &'a DossierTables,
    facts: &'a DossierFacts,
    member: &'a CastMember,
    slots: BTreeMap<String, String>,
    secrets: &'a [String],
    given_tags: BTreeSet<String>,
    /// Phrases from this NPC's own shame; empty until the flaw is rolled.
    shame: Vec<String>,
}
impl<'a> Context<'a> {
    fn new(
        tables: &'a DossierTables,
        facts: &'a DossierFacts,
        member: &'a CastMember,
        secrets: &'a [String],
    ) -> Self {
        let [sub, obj, pos] = member.gender.pronouns();
        let mut slots = BTreeMap::new();
        for (key, value) in [
            ("name", member.name.as_str()),
            ("role", member.role_label.as_str()),
            ("sub", sub),
            ("obj", obj),
            ("pos", pos),
            ("stake", facts.stake.short.as_str()),
            ("stake_full", facts.stake.full.as_str()),
            ("location", facts.location.as_str()),
            ("destination", facts.destination.as_str()),
        ] {
            slots.insert(key.to_owned(), value.to_owned());
        }
        slots.insert("role_a".into(), with_article(&member.role_label));
        // A slot naming the NPC itself stays unavailable, so no tie says "owes themselves".
        if member.role != NpcRole::Antagonist {
            slots.insert("antagonist".into(), facts.antagonist.short.clone());
            slots.insert("antagonist_the".into(), facts.antagonist.the.clone());
        }
        if member.role != NpcRole::Villain
            && let Some(villain) = &facts.villain
        {
            slots.insert("villain".into(), villain.clone());
        }
        if let Some(faction) = &facts.faction {
            slots.insert("faction".into(), faction.clone());
        }
        let given_tags = tables.social.tags_of(
            [&member.mannerism, &member.delivery, &member.voice]
                .into_iter()
                .flatten()
                .map(String::as_str),
        );
        Self {
            tables,
            facts,
            member,
            slots,
            secrets,
            given_tags,
            shame: Vec::new(),
        }
    }
    fn with_shame(&self, shame: Vec<String>) -> Self {
        Self {
            shame,
            ..self.clone()
        }
    }
    fn render(&self, template: &str) -> Option<String> {
        render::render(template, &self.slots).ok()
    }
    fn render_with(&self, template: &str, extra: &BTreeMap<String, String>) -> Option<String> {
        let mut slots = self.slots.clone();
        slots.extend(extra.clone());
        render::render(template, &slots).ok()
    }
    fn permitted(&self, text: &str, audience: Audience) -> bool {
        let hits = |terms: &[String]| terms.iter().any(|term| mentions_term(text, term));
        match audience {
            Audience::Public => !hits(self.secrets) && !hits(&self.shame),
            Audience::Trusted => !hits(&self.shame),
            Audience::DmOnly => true,
        }
    }
    /// Role, speaker and foe filters shared by every filtered table.
    fn admitted(&self, roles: &[NpcRole], speakers: &[Group], foes: &[Group]) -> bool {
        admits(roles, self.member.role)
            && admits(speakers, self.member.group)
            && admits(foes, self.facts.antagonist.group)
    }
    /// A profile fits when its filters admit this NPC and every field renders and is allowed
    /// in its audience. `fields` pairs each template with the audience that will hear it.
    fn profile_fits(&self, profile: &Profile, fields: &[(Option<&str>, Audience)]) -> bool {
        self.admitted(&profile.roles, &profile.speakers, &profile.foes)
            && !profile.except.contains(&self.member.archetype)
            && fields.iter().all(|(template, audience)| {
                template.is_none_or(|template| {
                    self.render(template)
                        .is_some_and(|text| self.permitted(&text, *audience))
                })
            })
    }
    fn clear_of_tags(&self, tags: &[String], taken: &BTreeSet<String>) -> bool {
        let wanted: BTreeSet<String> = tags.iter().cloned().collect();
        !clashing(&self.tables.social.incompatible, &wanted, taken)
    }
    fn other_candidates(&self, allowed: &[NpcRole]) -> Vec<&'a CastMember> {
        self.facts
            .cast
            .iter()
            .filter(|other| other.id != self.member.id && allowed.contains(&other.role))
            .collect()
    }
}

fn trait_of(profile: &Profile, text: String) -> Trait {
    Trait {
        id: profile.id.clone(),
        text,
    }
}
fn reaction_of(entry: &Profile, text: String, answer: String) -> Reaction {
    Reaction {
        id: entry.id.clone(),
        text,
        answer,
        tags: entry.tags.clone(),
    }
}
fn template_error(npc_id: &str, table: &str) -> DossierError {
    DossierError::Template {
        npc_id: npc_id.to_owned(),
        table: table.to_owned(),
    }
}
fn require(rendered: Option<String>, npc_id: &str, table: &str) -> Result<String, DossierError> {
    rendered.ok_or_else(|| template_error(npc_id, table))
}
fn field<'p>(
    value: &'p Option<String>,
    npc_id: &str,
    table: &str,
) -> Result<&'p str, DossierError> {
    value
        .as_deref()
        .ok_or_else(|| template_error(npc_id, table))
}

/// The template an NPC would say aloud for this entry, when it has one apart from its DM text.
fn spoken_template(profile: &Profile) -> Option<&str> {
    profile.confide.as_deref().or(profile.speak.as_deref())
}
/// The rendered line an NPC says for this entry (its plain text when it has no separate line).
fn spoken(context: &Context<'_>, profile: &Profile, table: &str) -> Result<String, DossierError> {
    let template = spoken_template(profile).unwrap_or(&profile.text);
    require(context.render(template), &context.member.id, table)
}

fn validate_facts(
    tables: &DossierTables,
    facts: &DossierFacts,
) -> Result<Vec<String>, DossierError> {
    if facts.cast.is_empty() {
        return Err(DossierError::EmptyCast);
    }
    if facts.cast.len() > MAX_CAST {
        return Err(DossierError::TooManyCast(facts.cast.len()));
    }
    let mut ids = BTreeSet::new();
    for member in &facts.cast {
        if member.id.is_empty() || member.name.is_empty() || member.role_label.is_empty() {
            return Err(DossierError::EmptyFact("cast member"));
        }
        if !ids.insert(member.id.as_str()) {
            return Err(DossierError::DuplicateNpc(member.id.clone()));
        }
        if !member.group.speaks() {
            return Err(DossierError::SilentSpeaker(member.id.clone()));
        }
    }
    for (name, value) in [
        ("stake.short", &facts.stake.short),
        ("stake.full", &facts.stake.full),
        ("location", &facts.location),
        ("location_id", &facts.location_id),
        ("destination", &facts.destination),
        ("antagonist.short", &facts.antagonist.short),
        ("antagonist.the", &facts.antagonist.the),
    ] {
        if value.is_empty() {
            return Err(DossierError::EmptyFact(name));
        }
    }
    let keywords = tables
        .knowledge
        .secret_keywords
        .get(&facts.secret_id)
        .ok_or_else(|| DossierError::UnknownSecret(facts.secret_id.clone()))?;
    let mut secrets: Vec<String> = facts
        .secret_terms
        .iter()
        .chain(keywords)
        .filter(|term| !term.trim().is_empty())
        .cloned()
        .collect();
    secrets.sort();
    secrets.dedup();
    Ok(secrets)
}

/// Roll one dossier per cast member. Pure: the same `(tables, facts, seed)` always returns the
/// same dossiers, in cast order, and the adventure's own roll stream is never touched.
pub(crate) fn roll_dossiers(
    tables: &DossierTables,
    facts: &DossierFacts,
    seed: u64,
) -> Result<Vec<Dossier>, DossierError> {
    let secrets = validate_facts(tables, facts)?;
    let mut used_ties = BTreeSet::new();
    let mut dossiers = Vec::with_capacity(facts.cast.len());
    for member in &facts.cast {
        let context = Context::new(tables, facts, member, &secrets);
        let dossier = roll_one(&context, seed, &mut used_ties)?;
        if let Some(leak) = dossier.leak() {
            return Err(DossierError::Leak {
                npc_id: dossier.npc_id,
                field: leak.field,
                term: leak.term,
            });
        }
        dossiers.push(dossier);
    }
    Ok(dossiers)
}

fn roll_one(
    base: &Context<'_>,
    seed: u64,
    used_ties: &mut BTreeSet<String>,
) -> Result<Dossier, DossierError> {
    let tables = base.tables;
    let member = base.member;
    let npc = member.id.as_str();
    let mut roller = Roller::new(seed, npc);
    let attitude = roller
        .pick(
            "attitude",
            tables.social.attitudes.for_role(member.role),
            |_| true,
        )?
        .attitude;

    // The flaw comes first so its shame terms can keep every later public line clean.
    let flaw = roller.pick("flaw", &tables.character.flaws, |entry| {
        base.profile_fits(
            entry,
            &[
                (Some(&entry.text), Audience::DmOnly),
                (entry.shame.as_deref(), Audience::DmOnly),
            ],
        )
    })?;
    let shame_text = require(base.render(field(&flaw.shame, npc, "flaw")?), npc, "flaw")?;
    let shame_terms = flaw.shame_terms.clone();
    let context = &base.with_shame(shame_terms.clone());

    let (background, character, ties) = (&tables.background, &tables.character, &tables.ties);
    let origin = roller.profile(context, "origin", &background.origins, Audience::Public)?;
    let trade = roller.profile(
        context,
        "past_trade",
        &background.past_trades,
        Audience::Public,
    )?;
    let event = roller.profile(
        context,
        "formative_event",
        &background.formative_events,
        Audience::Public,
    )?;
    let ideal = roller.profile(context, "ideal", &character.ideals, Audience::Public)?;
    let bond = roller.profile(context, "bond", &character.bonds, Audience::Public)?;
    let want = roller.profile(context, "want", &character.wants, Audience::Public)?;
    let fear = roller.profile(context, "fear", &character.fears, Audience::Trusted)?;
    let talent = roller.profile(context, "talent", &background.talents, Audience::DmOnly)?;

    // Speech style and temperament must agree with the traits the NPC already has.
    let mut taken = context.given_tags.clone();
    let style = roller.pick("speech_style", &tables.social.styles, |entry| {
        admits(&entry.speakers, member.group) && context.clear_of_tags(&entry.tags, &taken)
    })?;
    taken.extend(style.tags.iter().cloned());
    let social = &tables.social;
    let flattery = pick_reaction(
        context,
        &mut roller,
        "flattery",
        &social.flattery,
        &mut taken,
    )?;
    let flirting = pick_reaction(
        context,
        &mut roller,
        "flirting",
        &social.flirting,
        &mut taken,
    )?;
    let threats = pick_reaction(context, &mut roller, "threats", &social.threats, &mut taken)?;
    let lies = pick_reaction(context, &mut roller, "lies", &social.lies, &mut taken)?;
    let bribes = pick_reaction(context, &mut roller, "bribes", &social.bribes, &mut taken)?;
    let deflection = roller.pick("deflection", &social.deflections, |entry| {
        context.clear_of_tags(&entry.tags, &taken)
            && context.profile_fits(entry, &[(entry.speak.as_deref(), Audience::Public)])
    })?;

    let (story_tie, tie_cast) = roll_tie(context, &mut roller, used_ties)?;
    let opposing = member.role.opposes_party();
    let motive = if opposing {
        Some(roller.profile(context, "motive", &ties.motives, Audience::Public)?)
    } else {
        None
    };
    let stand_down = if opposing {
        Some(roller.profile(context, "stand_down", &ties.stand_downs, Audience::Trusted)?)
    } else {
        None
    };

    // Colour: a place line from the location and a generic one for everyone; the friendly cast
    // also knows a rumour, a hint to share once trusted, and a line about the wider world.
    let knowledge = &tables.knowledge;
    let specific = knowledge
        .place_specific
        .get(&context.facts.location_id)
        .ok_or_else(|| DossierError::NoCompatibleEntry {
            npc_id: npc.to_owned(),
            table: "place_specific".into(),
        })?;
    let place = roller.profile(context, "place_specific", specific, Audience::Public)?;
    let place_generic = roller.profile(
        context,
        "place_generic",
        &knowledge.place_generic,
        Audience::Public,
    )?;
    let (mut rumour, mut hint, mut world) = (None, None, None);
    if !opposing {
        rumour = Some(roller.profile(context, "rumour", &knowledge.rumours, Audience::Public)?);
        hint = Some(roller.profile(context, "hint", &knowledge.hints, Audience::Trusted)?);
        if context.facts.faction.is_some() || context.facts.villain.is_some() {
            world = Some(roller.profile(context, "world", &knowledge.world, Audience::Public)?);
        }
    }

    let knowledge = knowledge_tiers(
        context,
        &Picks {
            origin,
            trade,
            event,
            ideal,
            bond,
            want,
            fear,
            place,
            place_generic,
            motive,
            stand_down,
            rumour,
            hint,
            world,
        },
        &story_tie,
        shame_text,
        shame_terms,
    )?;
    let temperament = Temperament {
        flattery: reaction_entry(context, flattery)?,
        flirting: reaction_entry(context, flirting)?,
        threats: reaction_entry(context, threats)?,
        lies: reaction_entry(context, lies)?,
        bribes: reaction_entry(context, bribes)?,
    };
    let lines: BTreeMap<String, String> = knowledge
        .says_freely
        .iter()
        .map(|item| (item.source.clone(), item.text.clone()))
        .collect();
    let mut prepared_answers = BTreeMap::new();
    for topic in Topic::ALL {
        let body = match topic {
            Topic::ComplimentOrFlirt => temperament.flirting.answer.clone(),
            Topic::Threat => temperament.threats.answer.clone(),
            Topic::Bribe => temperament.bribes.answer.clone(),
            Topic::WhatAreYouHiding => spoken(context, deflection, "deflection")?,
            _ => frame_answer(context, &mut roller, topic, &lines)?,
        };
        prepared_answers.insert(topic, with_tic(topic, body, style, context));
    }

    let given = GivenTraits {
        mannerism: member.mannerism.clone(),
        delivery: member.delivery.clone(),
        voice: member.voice.clone(),
        tags: context.given_tags.iter().cloned().collect(),
        note: given_note(context),
    };
    let render_trait = |profile: &Profile, name: &str| -> Result<Trait, DossierError> {
        Ok(trait_of(
            profile,
            require(context.render(&profile.text), npc, name)?,
        ))
    };
    Ok(Dossier {
        npc_id: member.id.clone(),
        name: member.name.clone(),
        role: member.role,
        role_label: member.role_label.clone(),
        tables_revision: tables.revision.clone(),
        given,
        origin: render_trait(origin, "origin")?,
        past_trade: render_trait(trade, "past_trade")?,
        formative_event: render_trait(event, "formative_event")?,
        ideal: render_trait(ideal, "ideal")?,
        bond: render_trait(bond, "bond")?,
        flaw: render_trait(flaw, "flaw")?,
        want: render_trait(want, "want")?,
        fear: render_trait(fear, "fear")?,
        talent: render_trait(talent, "talent")?,
        story_tie: StoryTie {
            bound_cast: tie_cast,
            ..story_tie
        },
        motive: motive
            .map(|entry| render_trait(entry, "motive"))
            .transpose()?,
        stand_down: stand_down
            .map(|entry| render_trait(entry, "stand_down"))
            .transpose()?,
        knowledge,
        temperament,
        starting_attitude: attitude,
        speech_style: SpeechStyle {
            id: style.id.clone(),
            length: style.length,
            vocabulary: style.vocabulary.clone(),
            tic: style.tic.clone(),
            opener: style.opener.clone(),
            closer: style.closer.clone(),
            tags: style.tags.clone(),
        },
        prepared_answers,
        rolls: roller.rolls,
    })
}

/// The table entries that feed an NPC's knowledge tiers.
struct Picks<'t> {
    origin: &'t Profile,
    trade: &'t Profile,
    event: &'t Profile,
    ideal: &'t Profile,
    bond: &'t Profile,
    want: &'t Profile,
    fear: &'t Profile,
    place: &'t Profile,
    place_generic: &'t Profile,
    motive: Option<&'t Profile>,
    stand_down: Option<&'t Profile>,
    rumour: Option<&'t Profile>,
    hint: Option<&'t Profile>,
    world: Option<&'t Profile>,
}

/// Sort what the NPC knows into what is said freely, what waits for trust and what is never said.
fn knowledge_tiers(
    context: &Context<'_>,
    picks: &Picks<'_>,
    story_tie: &StoryTie,
    shame_text: String,
    shame_terms: Vec<String>,
) -> Result<KnowledgeTiers, DossierError> {
    let mut says_freely = vec![Knowledge {
        topic: Topic::WhoAreYou,
        source: "origin_line".into(),
        text: format!("I was {}.", spoken(context, picks.origin, "origin")?),
    }];
    let free_lines = [
        (
            Topic::WhoAreYou,
            "trade_line",
            Some(picks.trade),
            "past_trade",
        ),
        (
            Topic::YourPast,
            "event_line",
            Some(picks.event),
            "formative_event",
        ),
        (
            Topic::WhyThisMatters,
            "ideal_line",
            Some(picks.ideal),
            "ideal",
        ),
        (Topic::WhyThisMatters, "bond_line", Some(picks.bond), "bond"),
        (Topic::WhyThisMatters, "want_line", Some(picks.want), "want"),
        (Topic::WhyThisMatters, "motive_line", picks.motive, "motive"),
        (
            Topic::ThePlace,
            "place_line",
            Some(picks.place),
            "place_specific",
        ),
        (
            Topic::ThePlace,
            "place_line2",
            Some(picks.place_generic),
            "place_generic",
        ),
        (Topic::TheAntagonist, "rumour_line", picks.rumour, "rumour"),
        (Topic::ThePlace, "world_line", picks.world, "world"),
    ];
    for (topic, source, entry, table) in free_lines {
        if let Some(entry) = entry {
            says_freely.push(Knowledge {
                topic,
                source: source.into(),
                text: spoken(context, entry, table)?,
            });
        }
    }
    let tie_line = Knowledge {
        topic: Topic::WhyThisMatters,
        source: "tie_line".into(),
        text: story_tie.speak.clone(),
    };
    let mut says_when_trusted = Vec::new();
    if story_tie.public {
        says_freely.push(tie_line);
    } else {
        says_when_trusted.push(tie_line);
    }
    let trusted_lines = [
        (Topic::TheAntagonist, "hint_line", picks.hint, "hint"),
        (
            Topic::TheAntagonist,
            "stand_down_line",
            picks.stand_down,
            "stand_down",
        ),
        (
            Topic::WhatAreYouHiding,
            "fear_line",
            Some(picks.fear),
            "fear",
        ),
    ];
    for (topic, source, entry, table) in trusted_lines {
        if let Some(entry) = entry {
            says_when_trusted.push(Knowledge {
                topic,
                source: source.into(),
                text: spoken(context, entry, table)?,
            });
        }
    }
    let never_says = vec![
        Forbidden {
            source: "flaw_shame".into(),
            text: shame_text,
            terms: shame_terms,
            scope: ForbiddenScope::Everywhere,
        },
        Forbidden {
            source: "adventure_secret".into(),
            text: "The adventure's secret; its terms are listed here and are never confirmed \
                   or hinted at by this person."
                .into(),
            terms: context.secrets.to_vec(),
            scope: ForbiddenScope::PublicOnly,
        },
    ];
    Ok(KnowledgeTiers {
        says_freely,
        says_when_trusted,
        never_says,
    })
}

/// Pick one temperament entry that agrees with every tone already taken, then take its tone.
fn pick_reaction<'t>(
    context: &Context<'_>,
    roller: &mut Roller<'_>,
    name: &str,
    table: &'t Table<Profile>,
    taken: &mut BTreeSet<String>,
) -> Result<&'t Profile, DossierError> {
    let entry = roller.pick(name, table, |entry| {
        context.admitted(&entry.roles, &entry.speakers, &[])
            && context.clear_of_tags(&entry.tags, taken)
            && context.profile_fits(
                entry,
                &[
                    (Some(&entry.text), Audience::DmOnly),
                    (entry.speak.as_deref(), Audience::Public),
                ],
            )
    })?;
    taken.extend(entry.tags.iter().cloned());
    Ok(entry)
}

fn reaction_entry(context: &Context<'_>, entry: &Profile) -> Result<Reaction, DossierError> {
    let npc = context.member.id.as_str();
    Ok(reaction_of(
        entry,
        require(context.render(&entry.text), npc, "temperament")?,
        require(
            context.render(field(&entry.speak, npc, "temperament")?),
            npc,
            "temperament",
        )?,
    ))
}

/// Note for the DM when the generator's own traits already disagree (for example a brisk voice
/// beside a nervous mannerism).
fn given_note(context: &Context<'_>) -> Option<String> {
    let pairs = &context.tables.social.incompatible;
    pairs.iter().find_map(|[a, b]| {
        (context.given_tags.contains(a) && context.given_tags.contains(b)).then(|| {
            format!(
                "The given traits pull two ways ({a} against {b}). Play it as strain showing \
                 through, not as a contradiction."
            )
        })
    })
}

fn roll_tie(
    context: &Context<'_>,
    roller: &mut Roller<'_>,
    used_ties: &mut BTreeSet<String>,
) -> Result<(StoryTie, Option<String>), DossierError> {
    let npc = context.member.id.as_str();
    let table = context.tables.ties.for_role(context.member.role);
    // The "other" cast member is chosen by the stream, so it is fixed before the template is
    // judged; a template with no eligible other member is simply incompatible.
    let tie = roller.pick("story_tie", table, |tie| {
        !used_ties.contains(&tie.id)
            && context.admitted(&[], &tie.speakers, &tie.foes)
            && tie_slots(context, tie, None).is_some_and(|(text, speak)| {
                let public = if tie.public {
                    Audience::Public
                } else {
                    Audience::Trusted
                };
                context.permitted(&text, public) && context.permitted(&speak, public)
            })
    })?;
    used_ties.insert(tie.id.clone());
    let candidates = context.other_candidates(&tie.others);
    let other = if tie.others.is_empty() {
        None
    } else {
        let index = roller.pick_index("story_tie_other", candidates.len())?;
        candidates.get(index).copied()
    };
    let (text, speak) =
        tie_slots(context, tie, other).ok_or_else(|| template_error(npc, "story_tie"))?;
    let bound_fact =
        bound_fact_of(context, tie, other).ok_or_else(|| template_error(npc, "story_tie"))?;
    Ok((
        StoryTie {
            id: tie.id.clone(),
            binds: tie.binds,
            bound_fact,
            bound_cast: None,
            public: tie.public,
            text,
            speak,
        },
        other.map(|member| member.id.clone()),
    ))
}

/// Render a tie's two texts. With `other` absent, a tie that needs another member is judged by
/// whether any eligible member exists.
fn tie_slots(
    context: &Context<'_>,
    tie: &Tie,
    other: Option<&CastMember>,
) -> Option<(String, String)> {
    let mut extra = BTreeMap::new();
    if !tie.others.is_empty() {
        let member = match other {
            Some(member) => member,
            None => *context.other_candidates(&tie.others).first()?,
        };
        extra.insert("other".to_owned(), member.name.clone());
        extra.insert("other_role".to_owned(), member.role_label.clone());
    }
    Some((
        context.render_with(&tie.text, &extra)?,
        context.render_with(&tie.speak, &extra)?,
    ))
}
fn bound_fact_of(context: &Context<'_>, tie: &Tie, other: Option<&CastMember>) -> Option<String> {
    let used = slots_of(&tie.text).ok()?;
    let slot = tie.binds.slots().iter().find(|slot| used.contains(slot))?;
    match (*slot, other) {
        ("other", Some(member)) => Some(member.name.clone()),
        ("other", None) => None,
        (slot, _) => context.slots.get(slot).cloned(),
    }
}

/// Fill one frame for a topic from the lines the NPC may say freely.
fn frame_answer(
    context: &Context<'_>,
    roller: &mut Roller<'_>,
    topic: Topic,
    lines: &BTreeMap<String, String>,
) -> Result<String, DossierError> {
    let npc = context.member.id.as_str();
    let table = context
        .tables
        .social
        .frames
        .get(&topic)
        .ok_or_else(|| template_error(npc, "frames"))?;
    let mut slots = lines.clone();
    slots.insert("name".into(), context.member.name.clone());
    slots.insert("role_a".into(), with_article(&context.member.role_label));
    let frame = roller.pick(
        &format!("frame.{}", topic.label().replace(' ', "_")),
        table,
        |entry| {
            admits(&entry.roles, context.member.role)
                && render::render(&entry.text, &slots).is_ok_and(|text| {
                    sentence_count(&text) <= MAX_SENTENCES
                        && context.permitted(&text, Audience::Public)
                })
        },
    )?;
    render::render(&frame.text, &slots).map_err(|_| template_error(npc, "frames"))
}

/// Add the speech tic as an opener or a closer, alternating by topic, when it still fits.
fn with_tic(topic: Topic, body: String, style: &StyleEntry, context: &Context<'_>) -> String {
    // Even topics open with the tic and odd topics close with it, so answers do not all sound alike.
    let styled = if topic.ordinal().is_multiple_of(2) {
        format!("{} {body}", style.opener)
    } else {
        format!("{body} {}", style.closer)
    };
    let secret_free = !context
        .secrets
        .iter()
        .any(|term| mentions_term(&styled, term));
    if sentence_count(&styled) <= MAX_SENTENCES && secret_free {
        styled
    } else {
        body
    }
}

impl Dossier {
    /// Forbidden phrases that appear where they must not: shame anywhere spoken, secret terms
    /// in any public text.
    pub(crate) fn leak(&self) -> Option<Leak> {
        let mut public: Vec<(String, &str)> = self
            .knowledge
            .says_freely
            .iter()
            .map(|item| (format!("says_freely.{}", item.source), item.text.as_str()))
            .collect();
        for (topic, text) in &self.prepared_answers {
            public.push((format!("prepared_answers.{}", topic.label()), text));
        }
        let mut spoken: Vec<(String, &str)> = self
            .knowledge
            .says_when_trusted
            .iter()
            .map(|item| {
                (
                    format!("says_when_trusted.{}", item.source),
                    item.text.as_str(),
                )
            })
            .collect();
        spoken.push(("story_tie.text".into(), &self.story_tie.text));
        spoken.push(("story_tie.speak".into(), &self.story_tie.speak));
        if self.story_tie.public {
            public.push(("story_tie.text".into(), &self.story_tie.text));
            public.push(("story_tie.speak".into(), &self.story_tie.speak));
        }
        for forbidden in &self.knowledge.never_says {
            for term in &forbidden.terms {
                let hit = |places: &[(String, &str)]| {
                    places
                        .iter()
                        .find(|(_, text)| mentions_term(text, term))
                        .map(|(field, _)| Leak {
                            field: field.clone(),
                            term: term.clone(),
                        })
                };
                if let Some(leak) = hit(&public) {
                    return Some(leak);
                }
                if forbidden.scope == ForbiddenScope::Everywhere
                    && let Some(leak) = hit(&spoken)
                {
                    return Some(leak);
                }
            }
        }
        None
    }
}
