//! The server's NPC social layer: how a typed line is read, and how each NPC feels about each
//! hero. Governing design: adventure-generator section 19 and interaction-engine "Persistent
//! social and epistemic model".
//!
//! # What this is, and is not
//!
//! Attitude uses the standard 2024 Friendly / Indifferent / Hostile scale, and the server owns and
//! saves it per NPC and per hero. How it MOVES is a custom social layer: a small trust counter
//! that a warm or cool approach nudges, bounded as below. It is story colour. It grants no
//! bonus, changes no roll or difficulty and unlocks only conversation. It is NOT the standard
//! Influence action, which is an ability check that does not exist yet; when it does, it replaces
//! the way attitude moves and this layer only voices the outcome.
//!
//! # Bounds
//!
//! * An exchange moves trust by at most one point. Two points in one direction move the attitude
//!   one level and reset the counter, so one exchange moves attitude by at most one level.
//! * Per hero and NPC, the attitude never ends a scene more than one level from where the scene
//!   found it.
//! * An opposing NPC (antagonist, villain, lieutenant) never becomes Friendly from talk alone.
//!
//! The state is saved with the run in `social.json` (atomic replace) so a reload or reconnect
//! finds the same standing and the same recent conversation.

use std::{
    collections::BTreeMap,
    fs, io,
    mem::size_of,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
};

use df_protocol::common as rpc;
use df_types::MemberId;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use super::{
    adventure::dossier::{Attitude, Dossier, Effect, Topic},
    storyteller::contains_whole_word,
};

/// Trust points in one direction that move the attitude one level.
const TRUST_STEP: i8 = 2;
/// Lines kept, and shown to the model, of one hero's conversation with one NPC.
pub(super) const RECENT_LINES: usize = 6;
const RECENT_LINE_CHARACTERS: usize = 300;
const SCHEMA: &str = "df-npc-social/1";
const MAXIMUM_FILE_BYTES: u64 = 256 * 1024;
const FILE_NAME: &str = "social.json";

/// What a typed line does socially. Read by the model in the same call that writes the reply;
/// [`classify_keywords`] reads it when the model is off or does not answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Approach {
    Question,
    Kindness,
    Flattery,
    Flirting,
    Persuasion,
    Deception,
    Intimidation,
    Bribe,
    Insult,
    Other,
}

impl Approach {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Question => "question",
            Self::Kindness => "kindness",
            Self::Flattery => "flattery",
            Self::Flirting => "flirting",
            Self::Persuasion => "persuasion",
            Self::Deception => "deception",
            Self::Intimidation => "intimidation",
            Self::Bribe => "bribe",
            Self::Insult => "insult",
            Self::Other => "other",
        }
    }

    /// The model's one-word answer; anything else is not an approach.
    pub(super) fn parse(word: &str) -> Option<Self> {
        let word = word.trim().trim_matches(|c: char| !c.is_alphabetic());
        [
            Self::Question,
            Self::Kindness,
            Self::Flattery,
            Self::Flirting,
            Self::Persuasion,
            Self::Deception,
            Self::Intimidation,
            Self::Bribe,
            Self::Insult,
            Self::Other,
        ]
        .into_iter()
        .find(|approach| approach.label().eq_ignore_ascii_case(word))
    }
}

fn mentions_any(lowered: &str, phrases: &[&str]) -> bool {
    phrases
        .iter()
        .any(|phrase| contains_whole_word(lowered, phrase))
}

/// The keyword reading of a typed line, used when the model gives none. Strongest pressure first.
pub(super) fn classify_keywords(typed: &str) -> Approach {
    let lowered = typed.to_lowercase();
    let has = |phrases: &[&str]| mentions_any(&lowered, phrases);
    if has(&[
        "bribe",
        "gold",
        "coin",
        "coins",
        "silver",
        "pay you",
        "paid",
        "how much",
        "reward you",
    ]) {
        Approach::Bribe
    } else if has(&[
        "or else",
        "i will hurt",
        "i'll hurt",
        "kill you",
        "i will kill",
        "i'll kill",
        "break your",
        "you will regret",
        "you'll regret",
        "make you talk",
        "gut you",
        "cut your throat",
        "slit your throat",
        "torture you",
        "make you bleed",
        "threaten",
        "talk or",
        "i will end you",
    ]) {
        Approach::Intimidation
    } else if has(&[
        "idiot",
        "fool",
        "coward",
        "stupid",
        "worthless",
        "pathetic",
        "useless",
        "scum",
        "liar",
        "ugly",
        "disgusting",
    ]) {
        Approach::Insult
    } else if has(&[
        "beautiful",
        "handsome",
        "gorgeous",
        "charming",
        "kiss",
        "dinner with me",
        "sweetheart",
        "darling",
        "your eyes",
        "your smile",
        "fancy you",
        "sexy",
        "bed with me",
        "come to bed",
        "wink",
    ]) {
        Approach::Flirting
    } else if has(&[
        "admire",
        "impressive",
        "brilliant",
        "clever",
        "talented",
        "wonderful",
        "renowned",
        "famous",
        "legend",
        "wise",
        "wisest",
        "smartest",
        "bravest",
        "strongest",
        "amazing",
        "incredible",
        "magnificent",
        "great",
        "greatest",
    ]) {
        Approach::Flattery
    } else if has(&[
        "thank you",
        "thanks",
        "we will help",
        "we'll help",
        "here to help",
        "i will help",
        "i'll help",
        "don't worry",
        "do not worry",
        "are you alright",
        "are you okay",
        "i understand",
        "i'm sorry",
        "i am sorry",
        "must be hard",
        "must be difficult",
    ]) {
        Approach::Kindness
    } else if has(&[
        "i am the",
        "i'm the",
        "sent me",
        "i work for",
        "pretend",
        "i am a king",
        "royal",
    ]) {
        Approach::Deception
    } else if has(&[
        "trust me",
        "i promise",
        "believe me",
        "surely you",
        "please",
    ]) {
        Approach::Persuasion
    } else if typed.contains('?')
        || has(&[
            "who", "what", "why", "where", "how", "when", "tell me", "explain",
        ])
    {
        Approach::Question
    } else {
        Approach::Other
    }
}

/// The fixed topic a typed line is closest to, for the prepared answer. The approach takes
/// precedence where it names one (a compliment, a threat, a bribe).
pub(super) fn topic_of(typed: &str, approach: Approach) -> Topic {
    match approach {
        Approach::Flattery | Approach::Flirting => return Topic::ComplimentOrFlirt,
        Approach::Intimidation => return Topic::Threat,
        Approach::Bribe => return Topic::Bribe,
        _ => {}
    }
    let lowered = typed.to_lowercase();
    let has = |phrases: &[&str]| mentions_any(&lowered, phrases);
    if has(&[
        "hiding",
        "hide",
        "secret",
        "keeping",
        "tell me everything",
        "something you",
        "not telling",
    ]) {
        Topic::WhatAreYouHiding
    } else if has(&[
        "who are you",
        "your name",
        "yourself",
        "introduce",
        "what do you do",
        "your work",
        "your job",
    ]) {
        Topic::WhoAreYou
    } else if has(&[
        "past",
        "grew up",
        "childhood",
        "where are you from",
        "used to",
        "history",
        "before this",
        "your life",
        "your story",
    ]) {
        Topic::YourPast
    } else if has(&[
        "enemy",
        "villain",
        "behind this",
        "behind it",
        "threat",
        "danger",
        "attack",
        "hunting",
        "who is after",
        "who's after",
        "bandit",
        "goblin",
        "kobold",
        "monster",
        "creature",
        "beast",
        "undead",
    ]) {
        Topic::TheAntagonist
    } else if has(&[
        "town",
        "village",
        "here",
        "place",
        "area",
        "around",
        "this road",
        "the road",
        "local",
    ]) {
        Topic::ThePlace
    } else {
        Topic::WhyThisMatters
    }
}

fn rank(attitude: Attitude) -> i8 {
    match attitude {
        Attitude::Hostile => 0,
        Attitude::Indifferent => 1,
        Attitude::Friendly => 2,
    }
}

fn from_rank(rank: i8) -> Attitude {
    match rank {
        i8::MIN..=0 => Attitude::Hostile,
        1 => Attitude::Indifferent,
        _ => Attitude::Friendly,
    }
}

/// How a hero's last exchange moved an NPC.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Shift {
    /// The attitude changed level.
    Level { from: Attitude, to: Attitude },
    /// Only the trust counter moved, in the warm direction when `warmer`.
    Trust { warmer: bool },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Line {
    hero: bool,
    text: String,
}

/// One NPC's standing with one hero.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Relationship {
    attitude: Attitude,
    /// Between `-(TRUST_STEP - 1)` and `TRUST_STEP - 1` after every exchange.
    trust: i8,
    /// The scene the counters below describe; a new scene resets `scene_start`.
    scene: String,
    scene_start: Attitude,
    recent: Vec<Line>,
    /// Keys of the facts and traits this NPC has already offered or said to this hero, such as
    /// `trait:want` or `line:origin_line`. Kept across scenes so nothing is volunteered twice.
    #[serde(default)]
    voiced: Vec<String>,
    /// Exchanges in the current conversation: reset when the scene changes.
    #[serde(default)]
    exchanges: u32,
    /// Whether the NPC has said she does not know in the current conversation.
    #[serde(default)]
    admitted_ignorance: bool,
    /// Whether her last reply ended with a question, so the next should end another way.
    #[serde(default)]
    last_asked: bool,
}

impl Relationship {
    fn new(dossier: &Dossier) -> Self {
        Self {
            attitude: dossier.starting_attitude,
            trust: 0,
            scene: String::new(),
            scene_start: dossier.starting_attitude,
            recent: Vec::new(),
            voiced: Vec::new(),
            exchanges: 0,
            admitted_ignorance: false,
            last_asked: false,
        }
    }

    /// Applies one exchange's effect inside the bounds in the module documentation.
    fn apply(&mut self, scene: &str, effect: Effect, opposing: bool) -> Option<Shift> {
        if self.scene != scene {
            self.scene = scene.to_owned();
            self.scene_start = self.attitude;
            self.exchanges = 0;
            self.admitted_ignorance = false;
            self.last_asked = false;
        }
        let direction: i8 = match effect {
            Effect::Warm => 1,
            Effect::Cool => -1,
            Effect::Steady => return None,
        };
        let before = self.attitude;
        let trust = self.trust + direction;
        if trust.abs() < TRUST_STEP {
            self.trust = trust;
            return Some(Shift::Trust {
                warmer: direction > 0,
            });
        }
        let target = from_rank(rank(before) + direction);
        let within_scene = (rank(target) - rank(self.scene_start)).abs() <= 1;
        let allowed = within_scene && !(opposing && target == Attitude::Friendly);
        if target == before || !allowed {
            // At a limit: stay one point short of the next step.
            self.trust = direction * (TRUST_STEP - 1);
            return None;
        }
        self.attitude = target;
        self.trust = 0;
        Some(Shift::Level {
            from: before,
            to: target,
        })
    }
}

/// What a reply may draw on: the attitude and trust the hero has earned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Standing {
    pub(super) attitude: Attitude,
    pub(super) trust: i8,
}

impl Standing {
    /// Whether the trust-gated tier may be spoken: Friendly, or (opposing NPCs only, who cannot
    /// become Friendly) an Indifferent NPC who has started to listen.
    pub(super) fn trusted(self, dossier: &Dossier) -> bool {
        match self.attitude {
            Attitude::Friendly => true,
            Attitude::Indifferent => dossier.role.opposes_party() && self.trust >= 1,
            Attitude::Hostile => false,
        }
    }
}

/// What a reply may draw on from one conversation: its place in the exchanges, what this NPC has
/// already offered or said to this hero, and whether she has admitted she does not know.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct Conversation {
    /// This reply's number in the conversation, from 1.
    pub(super) exchange: u32,
    pub(super) voiced: Vec<String>,
    pub(super) admitted_ignorance: bool,
    pub(super) last_asked: bool,
}

/// Whether an NPC's line says she does not know or will not say. It reads the reply text, so
/// it counts the honest "I don't know" that the rules allow once per conversation. One
/// definition of a denial ([`super::speech_text::is_denial`]) decides everywhere: the prompt
/// rules, the speaking checks and this memory agree on what counted.
pub(super) fn admits_not_knowing(line: &str) -> bool {
    super::speech_text::is_denial(line)
}

/// The outcome of recording one exchange.
#[derive(Clone, Copy, Debug)]
pub(super) struct Recorded {
    pub(super) shift: Option<Shift>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SavedRelationship {
    member: String,
    npc: String,
    #[serde(flatten)]
    relationship: Relationship,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SavedFile {
    schema: String,
    adventure_seed: String,
    relationships: Vec<SavedRelationship>,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum SocialError {
    Io(io::ErrorKind),
    Invalid(&'static str),
}

impl std::fmt::Display for SocialError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(kind) => write!(formatter, "NPC social state file: {kind}"),
            Self::Invalid(reason) => write!(formatter, "NPC social state refused: {reason}"),
        }
    }
}

impl std::error::Error for SocialError {}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn member_from_hex(text: &str) -> Option<MemberId> {
    if text.len() != 32 {
        return None;
    }
    let mut bytes = [0_u8; 16];
    for (slot, pair) in bytes.iter_mut().zip(text.as_bytes().chunks(2)) {
        *slot = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    MemberId::from_bytes(&bytes).ok()
}

/// How the latest exchange landed, for presentation that follows the story's mood. It carries no
/// words: only who, what kind of approach, and where the NPC stands afterwards.
#[derive(Clone, Debug)]
pub(super) struct ExchangeRecord {
    /// Grows with every recorded exchange.
    pub(super) sequence: u64,
    pub(super) npc_id: String,
    pub(super) approach: Approach,
    pub(super) effect: Effect,
    pub(super) attitude: Attitude,
    pub(super) opposes: bool,
}

/// The standings and conversations of one saved moment, carried by the
/// development back/forward ring (DEVCTL-001) because the sidecar file keeps
/// no revision history of its own.
#[derive(Clone, Debug, Default)]
pub(super) struct SocialSnapshot {
    relationships: BTreeMap<(MemberId, String), Relationship>,
}

impl SocialSnapshot {
    pub(super) fn retained_heap_bytes(&self) -> Option<usize> {
        let mut bytes = self
            .relationships
            .len()
            .checked_mul(size_of::<((MemberId, String), Relationship)>())?;
        for ((_, npc), relationship) in &self.relationships {
            bytes = bytes.checked_add(npc.capacity())?;
            bytes = bytes.checked_add(
                relationship
                    .voiced
                    .capacity()
                    .checked_mul(size_of::<String>())?,
            )?;
            for key in &relationship.voiced {
                bytes = bytes.checked_add(key.capacity())?;
            }
        }
        Some(bytes)
    }
}

/// The standings and conversations of one run, and the dossiers that start them.
pub(super) struct SocialLedger {
    dossiers: Vec<Dossier>,
    adventure_seed: String,
    relationships: Mutex<BTreeMap<(MemberId, String), Relationship>>,
    file: Option<PathBuf>,
    /// Serialises saves so the newest snapshot is always the last one written.
    writing: Mutex<()>,
    /// The most recent exchange of the run (not saved: presentation only).
    latest: Mutex<Option<ExchangeRecord>>,
    /// Carries the latest exchange sequence so presentation can follow without polling.
    exchanges: watch::Sender<u64>,
}

impl SocialLedger {
    /// A ledger for a pack with no dossiers (the shipped harbor pack, or adventures saved before
    /// dossiers existed): nothing is addressable and nothing is saved.
    pub(super) fn none() -> Arc<Self> {
        Arc::new(Self {
            dossiers: Vec::new(),
            adventure_seed: String::new(),
            relationships: Mutex::new(BTreeMap::new()),
            file: None,
            writing: Mutex::new(()),
            latest: Mutex::new(None),
            exchanges: watch::channel(0).0,
        })
    }

    /// A ledger for an adventure's dossiers. When `directory` is given, standings saved there by
    /// an earlier run of the same adventure are loaded and later changes are saved there.
    pub(super) fn open(
        dossiers: Vec<Dossier>,
        adventure_seed: &str,
        directory: Option<PathBuf>,
    ) -> Result<Arc<Self>, SocialError> {
        let file = directory.map(|directory| directory.join(FILE_NAME));
        let mut relationships = BTreeMap::new();
        if let Some(file) = &file {
            match read_saved(file) {
                Ok(saved) => {
                    if saved.schema != SCHEMA || saved.adventure_seed != adventure_seed {
                        return Err(SocialError::Invalid("a different adventure's state"));
                    }
                    for entry in saved.relationships {
                        let member = member_from_hex(&entry.member)
                            .ok_or(SocialError::Invalid("member identity"))?;
                        let dossier = dossiers
                            .iter()
                            .find(|dossier| dossier.npc_id == entry.npc)
                            .ok_or(SocialError::Invalid("unknown NPC"))?;
                        let relationship = entry.relationship;
                        if relationship.trust.abs() >= TRUST_STEP
                            || relationship.recent.len() > RECENT_LINES
                            || (dossier.role.opposes_party()
                                && relationship.attitude == Attitude::Friendly
                                && dossier.starting_attitude != Attitude::Friendly)
                        {
                            return Err(SocialError::Invalid("standing out of bounds"));
                        }
                        relationships.insert((member, entry.npc), relationship);
                    }
                }
                Err(SocialError::Io(io::ErrorKind::NotFound)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(Arc::new(Self {
            dossiers,
            adventure_seed: adventure_seed.to_owned(),
            relationships: Mutex::new(relationships),
            file,
            writing: Mutex::new(()),
            latest: Mutex::new(None),
            exchanges: watch::channel(0).0,
        }))
    }

    pub(super) fn is_active(&self) -> bool {
        !self.dossiers.is_empty()
    }

    /// The dossier of a character on stage, by the pack's cast role and archetype id.
    pub(super) fn dossier_of(&self, role: rpc::SceneRole, id: &str) -> Option<&Dossier> {
        let label = match role {
            rpc::SceneRole::Patron => "patron",
            rpc::SceneRole::Contact => "contact",
            rpc::SceneRole::Antagonist => "antagonist",
            rpc::SceneRole::Bystander | rpc::SceneRole::Unspecified => return None,
        };
        let wanted = format!("{label}:{id}");
        self.dossiers
            .iter()
            .find(|dossier| dossier.npc_id == wanted)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<(MemberId, String), Relationship>> {
        self.relationships
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Snapshots every standing and conversation for a development rewind
    /// (DEVCTL-001): the sidecar file has no revision history, so the back /
    /// forward ring carries the saved moment's copy and restores all of it.
    pub(super) fn snapshot(&self) -> SocialSnapshot {
        SocialSnapshot {
            relationships: self.lock().clone(),
        }
    }

    /// Restores a snapshot taken by [`Self::snapshot`], in memory and in the
    /// sidecar file, so a rewound hero no longer knows lines from the future.
    /// Blocking file write like [`Self::save`]: call it from a blocking context
    /// (the session owner thread already blocks on durable commits there).
    pub(super) fn restore_snapshot(&self, snapshot: &SocialSnapshot) -> Result<(), SocialError> {
        *self.lock() = snapshot.relationships.clone();
        self.save()
    }

    /// What each present NPC has already told this hero: `(npc id, voiced keys)`,
    /// the same keys `social.json` keeps. Read surface for `dev state`, with room
    /// for the leads notes surface to join it later.
    pub(super) fn voiced_for(&self, member: MemberId) -> Vec<(String, Vec<String>)> {
        let mut told: Vec<(String, Vec<String>)> = self
            .lock()
            .iter()
            .filter(|((owner, _), _)| *owner == member)
            .map(|((_, npc), relationship)| (npc.clone(), relationship.voiced.clone()))
            .collect();
        told.sort();
        told
    }

    /// How this NPC feels about this hero right now.
    pub(super) fn standing(&self, member: MemberId, dossier: &Dossier) -> Standing {
        self.lock().get(&(member, dossier.npc_id.clone())).map_or(
            Standing {
                attitude: dossier.starting_attitude,
                trust: 0,
            },
            |relationship| Standing {
                attitude: relationship.attitude,
                trust: relationship.trust,
            },
        )
    }

    /// The last lines between this hero and this NPC, oldest first, as `(is_hero, text)`.
    pub(super) fn recent(&self, member: MemberId, npc_id: &str) -> Vec<(bool, String)> {
        self.lock()
            .get(&(member, npc_id.to_owned()))
            .map(|relationship| {
                relationship
                    .recent
                    .iter()
                    .map(|line| (line.hero, line.text.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// What this hero and NPC have already said in the conversation of `scene`.
    pub(super) fn conversation(
        &self,
        member: MemberId,
        dossier: &Dossier,
        scene: &str,
    ) -> Conversation {
        let relationships = self.lock();
        let Some(relationship) = relationships.get(&(member, dossier.npc_id.clone())) else {
            return Conversation {
                exchange: 1,
                ..Conversation::default()
            };
        };
        let continuing = relationship.scene == scene;
        Conversation {
            exchange: if continuing {
                relationship.exchanges.saturating_add(1)
            } else {
                1
            },
            voiced: relationship.voiced.clone(),
            admitted_ignorance: continuing && relationship.admitted_ignorance,
            last_asked: continuing && relationship.last_asked,
        }
    }

    /// Remembers the memory keys a reply offered as said to this hero. Called after the reply is
    /// recorded; the caller saves afterwards.
    pub(super) fn remember_offered(&self, member: MemberId, dossier: &Dossier, offered: &[String]) {
        let mut relationships = self.lock();
        let relationship = relationships
            .entry((member, dossier.npc_id.clone()))
            .or_insert_with(|| Relationship::new(dossier));
        for key in offered {
            if !relationship.voiced.contains(key) {
                relationship.voiced.push(key.clone());
            }
        }
    }

    /// Records one exchange: moves the standing by the approach's effect on this NPC, inside the
    /// bounds, and keeps the last lines of the conversation. The caller saves afterwards.
    pub(super) fn record_exchange(
        &self,
        member: MemberId,
        dossier: &Dossier,
        scene: &str,
        approach: Approach,
        hero_line: &str,
        npc_line: &str,
    ) -> Recorded {
        let mut relationships = self.lock();
        let relationship = relationships
            .entry((member, dossier.npc_id.clone()))
            .or_insert_with(|| Relationship::new(dossier));
        let shift = relationship.apply(
            scene,
            effect_of(dossier, approach),
            dossier.role.opposes_party(),
        );
        relationship.exchanges = relationship.exchanges.saturating_add(1);
        relationship.admitted_ignorance |= admits_not_knowing(npc_line);
        relationship.last_asked = npc_line.trim_end().ends_with('?');
        for (hero, text) in [(true, hero_line), (false, npc_line)] {
            relationship.recent.push(Line {
                hero,
                text: text.chars().take(RECENT_LINE_CHARACTERS).collect(),
            });
        }
        let excess = relationship.recent.len().saturating_sub(RECENT_LINES);
        relationship.recent.drain(..excess);
        let attitude = relationship.attitude;
        drop(relationships);
        let sequence = self.exchanges.borrow().saturating_add(1);
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(ExchangeRecord {
            sequence,
            npc_id: dossier.npc_id.clone(),
            approach,
            effect: effect_of(dossier, approach),
            attitude,
            opposes: dossier.role.opposes_party(),
        });
        self.exchanges.send_replace(sequence);
        Recorded { shift }
    }

    /// The latest exchange of the run, if any.
    pub(super) fn latest_exchange(&self) -> Option<ExchangeRecord> {
        self.latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Wakes when an exchange has been recorded.
    pub(super) fn subscribe_exchanges(&self) -> watch::Receiver<u64> {
        self.exchanges.subscribe()
    }

    /// Saves every standing and conversation, replacing the file atomically. A ledger without a
    /// file saves nothing. Blocking: call it from a blocking context.
    pub(super) fn save(&self) -> Result<(), SocialError> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let relationships = self
            .lock()
            .iter()
            .map(|((member, npc), relationship)| SavedRelationship {
                member: hex(member.as_bytes()),
                npc: npc.clone(),
                relationship: relationship.clone(),
            })
            .collect();
        let bytes = serde_json::to_vec_pretty(&SavedFile {
            schema: SCHEMA.to_owned(),
            adventure_seed: self.adventure_seed.clone(),
            relationships,
        })
        .map_err(|_| SocialError::Invalid("serialization"))?;
        write_atomically(file, &bytes).map_err(|error| SocialError::Io(error.kind()))
    }

    /// Adds the hero's own view of each present NPC's attitude to a player's view. The display's
    /// view gets nothing: attitude is per hero, and every change is already a public table line.
    pub(super) fn attach(&self, view: &mut rpc::ViewMessage, member: MemberId) {
        if !self.is_active() {
            return;
        }
        let Some(rpc::view_message::Audience::Player(player)) = view.audience.as_mut() else {
            return;
        };
        let (Some(scene), Some(journey)) = (player.scene.as_ref(), player.journey.as_mut()) else {
            return;
        };
        journey.standing = scene
            .cast
            .iter()
            .filter_map(|actor| {
                let role = rpc::SceneRole::try_from(actor.role).ok()?;
                let dossier = self.dossier_of(role, &actor.id)?;
                Some(rpc::NpcStanding {
                    npc_id: dossier.npc_id.clone(),
                    name: actor.name.clone(),
                    attitude: attitude_code(self.standing(member, dossier).attitude),
                })
            })
            .collect();
    }
}

/// The wire code of an attitude: 1 Hostile, 2 Indifferent, 3 Friendly (0 is unknown).
pub(super) fn attitude_code(attitude: Attitude) -> u32 {
    match attitude {
        Attitude::Hostile => 1,
        Attitude::Indifferent => 2,
        Attitude::Friendly => 3,
    }
}

fn effect_of(dossier: &Dossier, approach: Approach) -> Effect {
    let temperament = &dossier.temperament;
    match approach {
        Approach::Flattery => temperament.flattery.effect,
        Approach::Flirting => temperament.flirting.effect,
        Approach::Intimidation => temperament.threats.effect,
        Approach::Deception => temperament.lies.effect,
        Approach::Bribe => temperament.bribes.effect,
        Approach::Kindness => Effect::Warm,
        Approach::Insult => Effect::Cool,
        Approach::Question | Approach::Persuasion | Approach::Other => Effect::Steady,
    }
}

/// The public table line for a shift: "warms to Brakka" under the NPC's name.
pub(super) fn shift_line(shift: Shift, hero: &str) -> String {
    match shift {
        Shift::Level { from, to } if rank(to) > rank(from) => format!("warms to {hero}"),
        Shift::Level { .. } => format!("cools toward {hero}"),
        Shift::Trust { warmer: true } => format!("softens a little toward {hero}"),
        Shift::Trust { warmer: false } => format!("grows wary of {hero}"),
    }
}

fn read_saved(file: &PathBuf) -> Result<SavedFile, SocialError> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(file)
        .and_then(|opened| opened.take(MAXIMUM_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| SocialError::Io(error.kind()))?;
    if u64::try_from(bytes.len()).map_or(true, |length| length > MAXIMUM_FILE_BYTES) {
        return Err(SocialError::Invalid("file size"));
    }
    serde_json::from_slice(&bytes).map_err(|_| SocialError::Invalid("file contents"))
}

/// Writes a temporary file beside `file`, flushes it, renames it over `file` and flushes the
/// directory, so a crash leaves the old state or the new one, never a torn file.
/// Directory handles cannot be flushed this way on Windows, where the directory
/// flush is a documented no-op; on Unix it is unchanged.
fn write_atomically(file: &PathBuf, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let directory = file
        .parent()
        .ok_or_else(|| io::Error::other("social state has no directory"))?;
    let staging = directory.join(format!("{FILE_NAME}.tmp"));
    let mut staged = fs::File::create(&staging)?;
    staged.write_all(bytes)?;
    staged.sync_all()?;
    fs::rename(&staging, file)?;
    crate::file_identity::sync_directory(directory)
}

#[cfg(test)]
#[path = "social_tests.rs"]
mod tests;
