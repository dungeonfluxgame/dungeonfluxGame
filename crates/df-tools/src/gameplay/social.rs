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
//! * Ordinary approaches move attitude at most one level from the scene's starting point; each
//!   classified threat lowers it one level.
//! * A wrong the hero does (a threat, or an act the appraisal judges resentful) is kept as a
//!   grievance. Each one lowers attitude one level toward Hostile, whatever the scene, and holds
//!   resentment at least a little (much after a second). Recovery then waits for several warm
//!   exchanges, and an apology may soften wariness and fear but never clears a grievance or a
//!   permanent grudge.
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

use super::adventure::dossier::{
    AnswerSubject, Attitude, Dossier, Effect, EmotionalProfile, Forgiveness,
};

use super::adventure::scene_model::SceneTheftWitness;

#[path = "social_scene.rs"]
mod scene_state;

/// Trust points in one direction that move the attitude one level.
const TRUST_STEP: i8 = 2;
/// Lines kept, and shown to the model, of one hero's conversation with one NPC.
pub(super) const RECENT_LINES: usize = 6;
const RECENT_LINE_CHARACTERS: usize = 300;
const SCHEMA: &str = "df-npc-social/1";
const MAXIMUM_FILE_BYTES: u64 = 256 * 1024;
const FILE_NAME: &str = "social.json";
/// Lasting wrongs kept per hero and NPC; the oldest is forgotten past this bound.
const MAXIMUM_GRIEVANCES: usize = 8;
/// Theft attempts remembered per standing so a replayed journal is never recorded twice.
const MAXIMUM_WITNESSED_THEFTS: usize = 32;
/// Characters of a hero's own words kept in one grievance.
const GRIEVANCE_CHARACTERS: usize = 200;
/// Warm exchanges an NPC holding a grievance needs before trust counts toward recovery.
const MENDING_EXCHANGES: u8 = 3;
/// Try-on consents kept per hero and NPC; the oldest is forgotten past this bound.
const MAXIMUM_TRY_ON_CONSENTS: usize = 8;

/// A Jev-judged handover of coin in a spoken line (see
/// `approach_model::ApproachModel::judge_coin_handover`). `amount_cp` is the total in copper
/// pieces when Jev could name a count and denomination; `None` means coin changed hands but
/// the amount was not specified. This type carries no words and moves no money by itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CoinHandover {
    pub(super) amount_cp: Option<u32>,
}

/// One owner's consent that one hero may try on or handle one canonical item without buying
/// it. Stored per hero and per owner NPC in the social ledger; it grants no ownership.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct TryOnConsent {
    pub(super) item_id: String,
}

/// What a typed line does socially. Read by the model in the same call that writes the reply;
/// [`neutral_reading`] stands in when the model is off or does not answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Approach {
    Question,
    Kindness,
    Apology,
    Introduction,
    Refusal,
    Flattery,
    Flirting,
    Persuasion,
    Deception,
    Intimidation,
    Bribe,
    Bargain,
    Insult,
    Other,
}

impl Approach {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Question => "question",
            Self::Kindness => "kindness",
            Self::Apology => "apology",
            Self::Introduction => "introduction",
            Self::Refusal => "refusal",
            Self::Flattery => "flattery",
            Self::Flirting => "flirting",
            Self::Persuasion => "persuasion",
            Self::Deception => "deception",
            Self::Intimidation => "intimidation",
            Self::Bribe => "bribe",
            Self::Bargain => "bargain",
            Self::Insult => "insult",
            Self::Other => "other",
        }
    }

    pub(super) fn is_influence_attempt(self) -> bool {
        matches!(
            self,
            Self::Kindness
                | Self::Apology
                | Self::Flattery
                | Self::Flirting
                | Self::Persuasion
                | Self::Deception
                | Self::Intimidation
                | Self::Bribe
                | Self::Bargain
        )
    }

    /// The model's one-word answer; anything else is not an approach.
    pub(super) fn parse(word: &str) -> Option<Self> {
        let word = word.trim().trim_matches(|c: char| !c.is_alphabetic());
        [
            Self::Question,
            Self::Kindness,
            Self::Apology,
            Self::Introduction,
            Self::Refusal,
            Self::Flattery,
            Self::Flirting,
            Self::Persuasion,
            Self::Deception,
            Self::Intimidation,
            Self::Bribe,
            Self::Bargain,
            Self::Insult,
            Self::Other,
        ]
        .into_iter()
        .find(|approach| approach.label().eq_ignore_ascii_case(word))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ApproachReading {
    pub(super) approach: Approach,
    pub(super) subject: AnswerSubject,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum EmotionLevel {
    #[default]
    None,
    ALittle,
    Much,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum PatienceState {
    #[default]
    Plenty,
    Short,
    Done,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum EmotionReaction {
    Warmed,
    Charmed,
    Amused,
    Unmoved,
    Uneasy,
    Frightened,
    Offended,
    Disgusted,
    LosingPatience,
    Done,
    NeverForgive,
}

impl EmotionReaction {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Warmed => "warmed",
            Self::Charmed => "charmed",
            Self::Amused => "amused",
            Self::Unmoved => "unmoved",
            Self::Uneasy => "uneasy",
            Self::Frightened => "frightened",
            Self::Offended => "offended",
            Self::Disgusted => "disgusted",
            Self::LosingPatience => "losing_patience",
            Self::Done => "done_with_this_hero",
            Self::NeverForgive => "will_never_forgive_this",
        }
    }
}

impl EmotionLevel {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ALittle => "a_little",
            Self::Much => "much",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::None => 0,
            Self::ALittle => 1,
            Self::Much => 2,
        }
    }
}

/// The feelings a reaction can name in a public caption.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Feeling {
    Fond,
    Afraid,
    DrawnTo,
    Resentful,
    Wary,
}

impl Feeling {
    fn phrase(self) -> &'static str {
        match self {
            Self::Fond => "fond of",
            Self::Afraid => "afraid of",
            Self::DrawnTo => "drawn to",
            Self::Resentful => "resentful of",
            Self::Wary => "wary of",
        }
    }
}

impl EmotionReaction {
    /// The feeling this reaction moves, if any; amusement, patience and boredom name none.
    fn feeling(self) -> Option<Feeling> {
        match self {
            Self::Warmed => Some(Feeling::Fond),
            Self::Charmed => Some(Feeling::DrawnTo),
            Self::Uneasy => Some(Feeling::Wary),
            Self::Frightened => Some(Feeling::Afraid),
            Self::Offended | Self::Disgusted | Self::NeverForgive => Some(Feeling::Resentful),
            Self::Amused | Self::Unmoved | Self::LosingPatience | Self::Done => None,
        }
    }
}

impl PatienceState {
    fn label(self) -> &'static str {
        match self {
            Self::Plenty => "plenty",
            Self::Short => "short",
            Self::Done => "done",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum InfluenceWillingness {
    Willing,
    Hesitant,
    Unwilling,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) struct Appraisal {
    pub(super) reaction: EmotionReaction,
    pub(super) strength: EmotionLevel,
    pub(super) willingness: Option<InfluenceWillingness>,
    pub(super) effect: Effect,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct EmotionalState {
    pub(super) fond: EmotionLevel,
    pub(super) afraid: EmotionLevel,
    pub(super) drawn_to: EmotionLevel,
    pub(super) resentful: EmotionLevel,
    pub(super) wary: EmotionLevel,
    pub(super) patience: PatienceState,
    #[serde(default)]
    pub(super) never_forgives: Option<String>,
    #[serde(default)]
    pub(super) last_reaction: Option<EmotionReaction>,
    #[serde(default)]
    done_said: bool,
}

impl EmotionalState {
    fn apply(
        &mut self,
        appraisal: Appraisal,
        forgiveness: Forgiveness,
        grudge_reason: &str,
    ) -> EmotionReaction {
        let level = appraisal.strength;
        let reaction = if appraisal.reaction == EmotionReaction::NeverForgive
            && forgiveness != Forgiveness::Unforgiving
        {
            EmotionReaction::Offended
        } else {
            appraisal.reaction
        };
        match reaction {
            EmotionReaction::Warmed => self.fond = level,
            EmotionReaction::Charmed => {
                self.fond = level;
                self.drawn_to = level;
            }
            EmotionReaction::Amused | EmotionReaction::Unmoved => {}
            EmotionReaction::Uneasy => self.wary = level,
            EmotionReaction::Frightened => {
                self.afraid = level;
                self.wary = level;
            }
            EmotionReaction::Offended | EmotionReaction::Disgusted => {
                self.resentful = level;
            }
            EmotionReaction::LosingPatience => self.patience = PatienceState::Short,
            EmotionReaction::Done => {
                self.patience = PatienceState::Done;
                self.done_said = false;
            }
            EmotionReaction::NeverForgive => {
                self.resentful = EmotionLevel::Much;
                self.never_forgives = Some(grudge_reason.to_owned());
            }
        }
        self.last_reaction = Some(reaction);
        reaction
    }

    pub(super) fn summary(&self) -> String {
        format!(
            "fond={} afraid={} drawn_to={} resentful={} wary={} patience={} never_forgives={}",
            self.fond.label(),
            self.afraid.label(),
            self.drawn_to.label(),
            self.resentful.label(),
            self.wary.label(),
            self.patience.label(),
            self.never_forgives.is_some(),
        )
        .to_lowercase()
    }

    /// Only a visible change earns a public caption; small feelings stay in the relationship.
    pub(super) fn noticeable_change_from(&self, previous: &Self) -> bool {
        (self.patience != previous.patience && self.patience != PatienceState::Plenty)
            || (self.never_forgives.is_some() && previous.never_forgives.is_none())
            || [
                (self.fond, previous.fond),
                (self.afraid, previous.afraid),
                (self.drawn_to, previous.drawn_to),
                (self.resentful, previous.resentful),
                (self.wary, previous.wary),
            ]
            .into_iter()
            .any(|(now, before)| {
                (now == EmotionLevel::Much || before == EmotionLevel::Much) && now != before
            })
    }

    /// The public caption for the change this exchange made. It names the strongest feeling that
    /// moved this turn, preferring the feeling this turn's reaction names at equal strength, so an
    /// older mild feeling never stands in for it. The standing line is the fallback.
    pub(super) fn turn_line(&self, previous: &Self, npc: &str, hero: &str) -> String {
        if self.patience != previous.patience && self.patience == PatienceState::Done {
            return format!("{npc} will not answer {hero} again for now.");
        }
        if self.patience != previous.patience && self.patience == PatienceState::Short {
            return format!("{npc} is losing patience with {hero}.");
        }
        if self.never_forgives.is_some() && previous.never_forgives.is_none() {
            return format!("{npc} seems resentful of {hero}.");
        }
        let primary = self.last_reaction.and_then(EmotionReaction::feeling);
        let moved = [
            (Feeling::Resentful, self.resentful, previous.resentful),
            (Feeling::Afraid, self.afraid, previous.afraid),
            (Feeling::DrawnTo, self.drawn_to, previous.drawn_to),
            (Feeling::Fond, self.fond, previous.fond),
            (Feeling::Wary, self.wary, previous.wary),
        ]
        .into_iter()
        .filter(|(_, now, before)| now != before && *now != EmotionLevel::None)
        .max_by_key(|(feeling, now, _)| (now.rank(), Some(*feeling) == primary))
        .map(|(feeling, _, _)| feeling);
        match moved {
            Some(feeling) => format!("{npc} seems {} {hero}.", feeling.phrase()),
            None => self.perceived_line(npc, hero),
        }
    }

    pub(super) fn perceived_line(&self, npc: &str, hero: &str) -> String {
        if self.patience == PatienceState::Done {
            format!("{npc} will not answer {hero} again for now.")
        } else if self.patience == PatienceState::Short {
            format!("{npc} is losing patience with {hero}.")
        } else if self.never_forgives.is_some() || self.resentful == EmotionLevel::Much {
            format!("{npc} seems resentful of {hero}.")
        } else if self.afraid != EmotionLevel::None {
            format!("{npc} seems afraid of {hero}.")
        } else if self.drawn_to != EmotionLevel::None {
            format!("{npc} seems drawn to {hero}.")
        } else if self.fond != EmotionLevel::None {
            format!("{npc} seems fond of {hero}.")
        } else if self.wary != EmotionLevel::None {
            format!("{npc} seems wary of {hero}.")
        } else {
            format!("{npc} seems {} toward {hero}.", "guarded")
        }
    }
}

/// The neutral reading when Jev does not answer: no approach and no subject. The caller records
/// that the reading is neutral. Words never decide it.
pub(super) fn neutral_reading() -> ApproachReading {
    ApproachReading {
        approach: Approach::Other,
        subject: AnswerSubject::Other,
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

/// A wrong a hero did to this NPC that the NPC keeps. The words are the hero's own line, condensed
/// and bounded; only that hero's conversation ever recalls them.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Grievance {
    pub(super) act: GrievanceAct,
    pub(super) words: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum GrievanceAct {
    Threat,
    Wrong,
    /// A failed Sleight of Hand the hero was caught making against this NPC or the NPC's property.
    Theft,
}

impl Grievance {
    /// The grievance an exchange leaves: a threat always does, and any other approach does when
    /// the appraisal judged the hero's act resentful. Ordinary talk leaves none.
    fn from_exchange(
        approach: Approach,
        judged: Option<EmotionReaction>,
        hero_line: &str,
    ) -> Option<Self> {
        let act = if approach == Approach::Intimidation {
            GrievanceAct::Threat
        } else if matches!(
            judged,
            Some(
                EmotionReaction::Offended
                    | EmotionReaction::Disgusted
                    | EmotionReaction::NeverForgive
            )
        ) {
            GrievanceAct::Wrong
        } else {
            return None;
        };
        let words = hero_line
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(GRIEVANCE_CHARACTERS)
            .collect();
        Some(Self { act, words })
    }

    /// The line the NPC may recall to this hero, in the NPC's voice.
    fn recollection(&self) -> String {
        let deed = match self.act {
            GrievanceAct::Threat => "You threatened me",
            GrievanceAct::Wrong => "You wronged me",
            GrievanceAct::Theft => "You were caught trying to steal from me",
        };
        if self.words.is_empty() {
            format!("{deed}.")
        } else {
            format!("{deed}: \"{}\"", self.words)
        }
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
    /// Every NPC reply in the current scene, for sentence-level repeat checks.
    #[serde(default)]
    scene_said: Vec<String>,
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
    #[serde(default)]
    emotional: EmotionalState,
    #[serde(default)]
    last_subject: String,
    #[serde(default)]
    same_subject_count: u32,
    #[serde(default)]
    patience_spent: u16,
    /// Whether a hesitant Influence check against this hero failed in the current scene. The 2024
    /// rule holds a failed urging back for 24 hours; the scene is the nearest recorded span, so the
    /// same request is not rolled again until the scene changes.
    #[serde(default)]
    influence_failed: bool,
    /// Wrongs this hero did to this NPC, oldest first. They outlast every scene and restart.
    #[serde(default)]
    grievances: Vec<Grievance>,
    /// Operation IDs of theft attempts already recorded against this hero.
    #[serde(default)]
    witnessed_thefts: Vec<String>,
    /// Items this NPC, as owner/keeper, has consented for this hero to try on or handle.
    #[serde(default)]
    try_on_consents: Vec<TryOnConsent>,
    /// Warm exchanges counted toward recovery since the last grievance or attitude level gained.
    /// Only counted while grievances last; a grievance-holding NPC's trust waits on it.
    #[serde(default)]
    mended: u8,
    /// Set when a threat has already moved the attitude for the exchange now being answered, so
    /// recording that exchange does not move it again. Never stored.
    #[serde(skip)]
    threat_applied: bool,
    #[serde(skip)]
    appraised: bool,
    #[serde(skip)]
    pending_shift: Option<Shift>,
}

impl Relationship {
    fn new(dossier: &Dossier) -> Self {
        Self {
            attitude: dossier.starting_attitude,
            trust: 0,
            scene: String::new(),
            scene_start: dossier.starting_attitude,
            recent: Vec::new(),
            scene_said: Vec::new(),
            voiced: Vec::new(),
            exchanges: 0,
            admitted_ignorance: false,
            last_asked: false,
            emotional: EmotionalState::default(),
            last_subject: String::new(),
            same_subject_count: 0,
            patience_spent: 0,
            influence_failed: false,
            grievances: Vec::new(),
            witnessed_thefts: Vec::new(),
            try_on_consents: Vec::new(),
            mended: 0,
            threat_applied: false,
            appraised: false,
            pending_shift: None,
        }
    }

    fn begin_scene(&mut self, scene: &str) {
        if self.scene == scene {
            return;
        }
        self.scene = scene.to_owned();
        self.scene_start = self.attitude;
        self.exchanges = 0;
        self.scene_said.clear();
        self.admitted_ignorance = false;
        self.last_asked = false;
        self.emotional.patience = PatienceState::Plenty;
        self.emotional.done_said = false;
        self.last_subject.clear();
        self.same_subject_count = 0;
        self.patience_spent = 0;
        self.influence_failed = false;
    }

    /// Applies one exchange's effect inside the bounds in the module documentation.
    fn apply(&mut self, scene: &str, effect: Effect, opposing: bool) -> Option<Shift> {
        self.begin_scene(scene);
        let direction: i8 = match effect {
            Effect::Warm => 1,
            Effect::Cool => -1,
            Effect::Steady => return None,
        };
        if direction > 0 && !self.grievances.is_empty() {
            // A held wrong slows recovery: warm moments must accumulate before trust counts.
            self.mended = self.mended.saturating_add(1).min(MENDING_EXCHANGES);
            if self.mended < MENDING_EXCHANGES {
                return None;
            }
        }
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
        self.mended = 0;
        Some(Shift::Level {
            from: before,
            to: target,
        })
    }

    /// Moves the attitude one level toward hostile, at most to Hostile, and clears the trust.
    fn lower_attitude(&mut self) -> Option<Shift> {
        let before = self.attitude;
        let target = from_rank(rank(before) - 1);
        if target == before {
            return None;
        }
        self.attitude = target;
        self.trust = 0;
        Some(Shift::Level {
            from: before,
            to: target,
        })
    }

    fn threaten(&mut self, scene: &str) -> Option<Shift> {
        self.begin_scene(scene);
        self.threat_applied = true;
        self.lower_attitude()
    }

    /// Keeps a wrong for good. The first wrong makes resentment at least a little; a second makes
    /// it much. Unless this exchange already lowered the attitude, the wrong lowers it one level.
    /// Recovery restarts from nothing.
    fn record_grievance(&mut self, grievance: Grievance, already_lowered: bool) -> Option<Shift> {
        self.grievances.push(grievance);
        let resentment = if self.grievances.len() > 1 {
            EmotionLevel::Much
        } else {
            EmotionLevel::ALittle
        };
        let excess = self.grievances.len().saturating_sub(MAXIMUM_GRIEVANCES);
        self.grievances.drain(..excess);
        if self.emotional.resentful.rank() < resentment.rank() {
            self.emotional.resentful = resentment;
        }
        self.mended = 0;
        self.trust = 0;
        if already_lowered {
            None
        } else {
            self.lower_attitude()
        }
    }

    /// Keeps one caught theft, once per attempt. Whoever keeps it gets the grievance; the owner
    /// also turns Hostile and resents the hero most, and never forgives where the owner's profile
    /// allows it. Returns whether the attempt was new.
    fn record_theft(&mut self, operation: &str, item: &str, keeper: TheftKeeper) -> bool {
        if self.witnessed_thefts.iter().any(|known| known == operation) {
            return false;
        }
        self.witnessed_thefts.push(operation.to_owned());
        let excess = self
            .witnessed_thefts
            .len()
            .saturating_sub(MAXIMUM_WITNESSED_THEFTS);
        self.witnessed_thefts.drain(..excess);
        let words: String = item.chars().take(GRIEVANCE_CHARACTERS).collect();
        self.record_grievance(
            Grievance {
                act: GrievanceAct::Theft,
                words,
            },
            false,
        );
        if let TheftKeeper::Owner { unforgiving } = keeper {
            self.attitude = Attitude::Hostile;
            self.emotional.resentful = EmotionLevel::Much;
            if unforgiving {
                self.emotional.never_forgives = Some(format!("caught stealing {item}"));
            }
        }
        true
    }

    /// Keeps one try-on consent for this hero, once per item. Returns whether it was new.
    #[allow(dead_code)] // Called through the ledger consent API (see its note).
    fn record_try_on_consent(&mut self, item_id: &str) -> bool {
        if self
            .try_on_consents
            .iter()
            .any(|consent| consent.item_id == item_id)
        {
            return false;
        }
        self.try_on_consents.push(TryOnConsent {
            item_id: item_id.to_owned(),
        });
        let excess = self
            .try_on_consents
            .len()
            .saturating_sub(MAXIMUM_TRY_ON_CONSENTS);
        self.try_on_consents.drain(..excess);
        true
    }
}

/// Who keeps a witnessed theft: the owner, or a different person who noticed the attempt.
#[derive(Clone, Copy)]
enum TheftKeeper {
    Owner { unforgiving: bool },
    Noticer,
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
    /// Every NPC reply in this scene, including lines outside the recent prompt window.
    pub(super) said: Vec<String>,
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
    pub(super) reaction: Option<EmotionReaction>,
    pub(super) emotional_state: EmotionalState,
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
            bytes = bytes.checked_add(
                relationship
                    .grievances
                    .capacity()
                    .checked_mul(size_of::<Grievance>())?,
            )?;
            for grievance in &relationship.grievances {
                bytes = bytes.checked_add(grievance.words.capacity())?;
            }
        }
        Some(bytes)
    }
}

/// The standings and conversations of one run, and the dossiers that start them.
pub(super) struct SocialLedger {
    rooms: Mutex<scene_state::Rooms>,
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
            rooms: Mutex::default(),
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
        let rooms = scene_state::Rooms::default();
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
                            || relationship.grievances.len() > MAXIMUM_GRIEVANCES
                            || relationship.grievances.iter().any(|grievance| {
                                grievance.words.chars().count() > GRIEVANCE_CHARACTERS
                            })
                            || relationship.mended > MENDING_EXCHANGES
                            || relationship.try_on_consents.len() > MAXIMUM_TRY_ON_CONSENTS
                            || relationship.last_subject.len() > 512
                            || relationship.same_subject_count > 100
                            || relationship.patience_spent > 100
                            || relationship.emotional.never_forgives.as_ref().is_some_and(
                                |reason| {
                                    reason.is_empty()
                                        || reason.chars().count() > 280
                                        || dossier.emotional_profile().forgiveness
                                            != Forgiveness::Unforgiving
                                },
                            )
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
        let ledger = Arc::new(Self {
            rooms: Mutex::new(rooms),
            dossiers,
            adventure_seed: adventure_seed.to_owned(),
            relationships: Mutex::new(relationships),
            file,
            writing: Mutex::new(()),
            latest: Mutex::new(None),
            exchanges: watch::channel(0).0,
        });
        ledger.install_cached_rooms()?;
        Ok(ledger)
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
            rpc::SceneRole::Bystander => "bystander",
            rpc::SceneRole::Unspecified => return None,
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
        {
            let mut rooms = self.rooms.lock().unwrap_or_else(PoisonError::into_inner);
            rooms.projections.clear();
            rooms.active_scene = None;
        }
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

    /// Only this hero's observable permanent breaks, for the owning notebook's private notes.
    pub(super) fn permanent_grievances(&self, member: MemberId) -> Vec<(String, String)> {
        self.lock()
            .iter()
            .filter(|((owner, _), _)| *owner == member)
            .filter_map(|((_, npc_id), relationship)| {
                let _reason = relationship.emotional.never_forgives.as_ref()?;
                let dossier = self
                    .dossiers
                    .iter()
                    .find(|dossier| dossier.npc_id == *npc_id)?;
                Some((
                    dossier.name.clone(),
                    "They made it clear that this offense will not be forgiven.".to_owned(),
                ))
            })
            .collect()
    }

    /// The wrongs this hero did to this NPC, oldest first, as lines the NPC recalls in its own
    /// voice. Only this hero's record is read, so no other hero hears them.
    pub(super) fn grievance_lines(&self, member: MemberId, dossier: &Dossier) -> Vec<String> {
        self.lock()
            .get(&(member, dossier.npc_id.clone()))
            .map_or_else(Vec::new, |relationship| {
                relationship
                    .grievances
                    .iter()
                    .map(Grievance::recollection)
                    .collect()
            })
    }

    /// Sets one hero's attitude and/or trust with one NPC for a development edit. The values must
    /// be a standing the dossier can hold: trust within one step either way, and no friendliness
    /// for an opposing NPC that does not start friendly. A refused edit changes nothing.
    pub(super) fn dev_set_standing(
        &self,
        member: MemberId,
        dossier: &Dossier,
        attitude: Option<Attitude>,
        trust: Option<i8>,
    ) -> Result<Standing, SocialError> {
        if trust.is_some_and(|trust| trust.abs() >= TRUST_STEP) {
            return Err(SocialError::Invalid("trust is out of range"));
        }
        let mut relationships = self.lock();
        let key = (member, dossier.npc_id.clone());
        let current = relationships.get(&key).map_or(
            Standing {
                attitude: dossier.starting_attitude,
                trust: 0,
            },
            |relationship| Standing {
                attitude: relationship.attitude,
                trust: relationship.trust,
            },
        );
        let next_attitude = attitude.unwrap_or(current.attitude);
        if next_attitude == Attitude::Friendly
            && dossier.role.opposes_party()
            && dossier.starting_attitude != Attitude::Friendly
        {
            return Err(SocialError::Invalid(
                "an opposing NPC does not become friendly",
            ));
        }
        let relationship = relationships
            .entry(key)
            .or_insert_with(|| Relationship::new(dossier));
        relationship.attitude = next_attitude;
        relationship.trust = trust.unwrap_or(current.trust);
        Ok(Standing {
            attitude: relationship.attitude,
            trust: relationship.trust,
        })
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

    /// The bounded relationship feelings for this hero; changing scenes resets only patience.
    pub(super) fn emotional_state(
        &self,
        member: MemberId,
        dossier: &Dossier,
        scene: &str,
    ) -> EmotionalState {
        let mut relationships = self.lock();
        let relationship = relationships
            .entry((member, dossier.npc_id.clone()))
            .or_insert_with(|| Relationship::new(dossier));
        relationship.begin_scene(scene);
        relationship.emotional.clone()
    }

    #[cfg(test)]
    fn note_subject(
        &self,
        member: MemberId,
        dossier: &Dossier,
        scene: &str,
        subject: AnswerSubject,
    ) -> u32 {
        self.note_question(member, dossier, scene, subject.label())
    }

    /// Counts similar asks by the actual question, not by broad labels like "people".
    pub(super) fn note_question(
        &self,
        member: MemberId,
        dossier: &Dossier,
        scene: &str,
        words: &str,
    ) -> u32 {
        let key: String = super::speech_text::normalised(words)
            .chars()
            .take(128)
            .collect();
        let mut relationships = self.lock();
        let relationship = relationships
            .entry((member, dossier.npc_id.clone()))
            .or_insert_with(|| Relationship::new(dossier));
        relationship.begin_scene(scene);
        if relationship.last_subject == key {
            relationship.same_subject_count =
                relationship.same_subject_count.saturating_add(1).min(100);
        } else {
            relationship.last_subject = key;
            relationship.same_subject_count = 1;
        }
        relationship.same_subject_count.saturating_sub(1)
    }

    /// Keyless fallback uses the dossier's reactions and patience; it invents no model-only
    /// attraction, fear or permanent grudge. Repetition costs more; a helpful line restores some.
    pub(super) fn fallback_appraisal(
        &self,
        member: MemberId,
        dossier: &Dossier,
        scene: &str,
        approach: Approach,
        similar_asks: u32,
    ) -> Appraisal {
        let effect = effect_of(dossier, approach);
        let mut relationships = self.lock();
        let relationship = relationships
            .entry((member, dossier.npc_id.clone()))
            .or_insert_with(|| Relationship::new(dossier));
        relationship.begin_scene(scene);
        let emotion = &relationship.emotional;
        let profile = dossier.emotional_profile();
        let base: u16 = match profile.patience {
            super::adventure::dossier::Patience::Low => 7,
            super::adventure::dossier::Patience::Middling => 16,
            super::adventure::dossier::Patience::High => 24,
        };
        let variation = dossier
            .npc_id
            .bytes()
            .fold(0_u16, |sum, byte| sum.wrapping_add(u16::from(byte)))
            % 3;
        let fond = match emotion.fond {
            EmotionLevel::None => 0,
            EmotionLevel::ALittle => 3,
            EmotionLevel::Much => 6,
        };
        let attitude = match relationship.attitude {
            Attitude::Friendly => 3,
            Attitude::Indifferent => 0,
            Attitude::Hostile => 0,
        };
        let adverse = if emotion.afraid == EmotionLevel::Much
            || emotion.resentful == EmotionLevel::Much
        {
            6
        } else if emotion.afraid != EmotionLevel::None || emotion.resentful != EmotionLevel::None {
            3
        } else {
            0
        };
        let limit = (base + variation + fond + attitude)
            .saturating_sub(adverse)
            .max(4);
        if effect == Effect::Warm {
            relationship.patience_spent = relationship.patience_spent.saturating_sub(3);
        } else {
            let cost = 1 + u16::from(similar_asks > 0) * 2 + u16::from(effect == Effect::Cool);
            relationship.patience_spent = relationship.patience_spent.saturating_add(cost).min(100);
        }
        let reaction = if relationship.patience_spent >= limit {
            EmotionReaction::Done
        } else if relationship.patience_spent >= limit.saturating_sub(3) {
            EmotionReaction::LosingPatience
        } else {
            if effect == Effect::Warm {
                relationship.emotional.patience = PatienceState::Plenty;
            }
            match effect {
                Effect::Warm => EmotionReaction::Warmed,
                Effect::Cool => EmotionReaction::Offended,
                Effect::Steady => EmotionReaction::Unmoved,
            }
        };
        Appraisal {
            reaction,
            strength: EmotionLevel::ALittle,
            willingness: None,
            effect,
        }
    }

    /// Whether this hero has already heard the character say it will not answer again this scene.
    pub(super) fn done_for(&self, member: MemberId, dossier: &Dossier, scene: &str) -> bool {
        let mut relationships = self.lock();
        let relationship = relationships
            .entry((member, dossier.npc_id.clone()))
            .or_insert_with(|| Relationship::new(dossier));
        relationship.begin_scene(scene);
        relationship.emotional.patience == PatienceState::Done && relationship.emotional.done_said
    }

    /// Whether a hesitant Influence check this hero made against this NPC already failed in this
    /// scene, so the same urging is refused without another roll.
    pub(super) fn influence_failed_for(
        &self,
        member: MemberId,
        dossier: &Dossier,
        scene: &str,
    ) -> bool {
        let mut relationships = self.lock();
        let relationship = relationships
            .entry((member, dossier.npc_id.clone()))
            .or_insert_with(|| Relationship::new(dossier));
        relationship.begin_scene(scene);
        relationship.influence_failed
    }

    /// Records that a hesitant Influence check failed, held until the scene changes. The reply's
    /// save persists it with the rest of the standing.
    pub(super) fn record_influence_failure(
        &self,
        member: MemberId,
        dossier: &Dossier,
        scene: &str,
    ) {
        let mut relationships = self.lock();
        let relationship = relationships
            .entry((member, dossier.npc_id.clone()))
            .or_insert_with(|| Relationship::new(dossier));
        relationship.begin_scene(scene);
        relationship.influence_failed = true;
    }

    /// Commits Jev's selected reaction before the reply is formed.
    pub(super) fn apply_appraisal(
        &self,
        member: MemberId,
        dossier: &Dossier,
        scene: &str,
        appraisal: Appraisal,
    ) -> (EmotionReaction, Option<Shift>) {
        let profile = dossier.emotional_profile();
        let grudge_reason = profile
            .feelings
            .iter()
            .find(|feeling| feeling.kind == super::adventure::dossier::FeelingKind::Grudge)
            .map_or("that offense".to_owned(), |feeling| feeling.text.clone());
        let mut relationships = self.lock();
        let relationship = relationships
            .entry((member, dossier.npc_id.clone()))
            .or_insert_with(|| Relationship::new(dossier));
        relationship.begin_scene(scene);
        // Forgiving recovery follows several meaningful warm exchanges, using the same bounded
        // trust already earned, or the mending count once a wrong is held. A lasting grudge is
        // never cleared here, and a held wrong keeps resentment at least a little.
        let lasting_wrong = !relationship.grievances.is_empty();
        let mended_enough = if lasting_wrong {
            relationship.mended >= MENDING_EXCHANGES
        } else {
            relationship.trust >= TRUST_STEP - 1
        };
        if profile.forgiveness == Forgiveness::Forgiving
            && appraisal.reaction == EmotionReaction::Warmed
            && appraisal.effect == Effect::Warm
            && mended_enough
            && relationship.emotional.never_forgives.is_none()
        {
            for feeling in [
                &mut relationship.emotional.afraid,
                &mut relationship.emotional.wary,
            ] {
                *feeling = match *feeling {
                    EmotionLevel::None | EmotionLevel::ALittle => EmotionLevel::None,
                    EmotionLevel::Much => EmotionLevel::ALittle,
                };
            }
            relationship.emotional.resentful = match relationship.emotional.resentful {
                EmotionLevel::None => EmotionLevel::None,
                EmotionLevel::ALittle if lasting_wrong => EmotionLevel::ALittle,
                EmotionLevel::ALittle => EmotionLevel::None,
                EmotionLevel::Much => EmotionLevel::ALittle,
            };
        }
        let reaction = relationship
            .emotional
            .apply(appraisal, profile.forgiveness, &grudge_reason);
        let shift = if reaction == EmotionReaction::NeverForgive
            || relationship.emotional.never_forgives.is_some()
        {
            let before = relationship.attitude;
            relationship.attitude = Attitude::Hostile;
            relationship.trust = 0;
            (before != Attitude::Hostile).then_some(Shift::Level {
                from: before,
                to: Attitude::Hostile,
            })
        } else {
            relationship.apply(scene, appraisal.effect, dossier.role.opposes_party())
        };
        relationship.appraised = true;
        relationship.pending_shift = shift;
        (reaction, shift)
    }

    /// Preserves the existing threat rule while recording its old temperament reaction.
    pub(super) fn apply_legacy_threat_emotion(
        &self,
        member: MemberId,
        dossier: &Dossier,
        scene: &str,
    ) -> EmotionReaction {
        let appraisal = self.fallback_appraisal(member, dossier, scene, Approach::Intimidation, 0);
        let profile = dossier.emotional_profile();
        let mut relationships = self.lock();
        let relationship = relationships
            .entry((member, dossier.npc_id.clone()))
            .or_insert_with(|| Relationship::new(dossier));
        relationship.begin_scene(scene);
        relationship
            .emotional
            .apply(appraisal, profile.forgiveness, "that threat")
    }

    /// Development-only state keyed by opaque member ids; the caller resolves ids to hero names.
    pub(super) fn development_feelings(
        &self,
        npc_id: &str,
    ) -> Option<(EmotionalProfile, Vec<(String, Attitude, EmotionalState)>)> {
        let dossier = self
            .dossiers
            .iter()
            .find(|dossier| dossier.npc_id == npc_id)?;
        let relationships = self.lock();
        let mut states: Vec<_> = relationships
            .iter()
            .filter(|((_, npc), _)| npc == npc_id)
            .map(|((member, _), relationship)| {
                (
                    hex(member.as_bytes()),
                    relationship.attitude,
                    relationship.emotional.clone(),
                )
            })
            .collect();
        states.sort_by(|left, right| left.0.cmp(&right.0));
        Some((dossier.emotional_profile(), states))
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
            said: if continuing {
                relationship.scene_said.clone()
            } else {
                Vec::new()
            },
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
        let appraised = std::mem::take(&mut relationship.appraised);
        let threatened = std::mem::take(&mut relationship.threat_applied);
        let mut shift = if appraised {
            relationship.pending_shift.take()
        } else if threatened {
            None
        } else {
            relationship.apply(
                scene,
                effect_of(dossier, approach),
                dossier.role.opposes_party(),
            )
        };
        let judged = if appraised {
            relationship.emotional.last_reaction
        } else {
            None
        };
        if let Some(grievance) = Grievance::from_exchange(approach, judged, hero_line) {
            let lowered = threatened
                || matches!(shift, Some(Shift::Level { from, to }) if rank(to) < rank(from));
            shift = relationship.record_grievance(grievance, lowered).or(shift);
        }
        relationship.exchanges = relationship.exchanges.saturating_add(1);
        if relationship.emotional.patience == PatienceState::Done {
            relationship.emotional.done_said = true;
        }
        relationship.admitted_ignorance |= admits_not_knowing(npc_line);
        relationship.last_asked = npc_line.trim_end().ends_with('?');
        relationship
            .scene_said
            .push(npc_line.chars().take(RECENT_LINE_CHARACTERS).collect());
        for (hero, text) in [(true, hero_line), (false, npc_line)] {
            relationship.recent.push(Line {
                hero,
                text: text.chars().take(RECENT_LINE_CHARACTERS).collect(),
            });
        }
        let excess = relationship.recent.len().saturating_sub(RECENT_LINES);
        relationship.recent.drain(..excess);
        let attitude = relationship.attitude;
        let reaction = relationship.emotional.last_reaction;
        let emotional_state = relationship.emotional.clone();
        drop(relationships);
        let sequence = self.exchanges.borrow().saturating_add(1);
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(ExchangeRecord {
            sequence,
            npc_id: dossier.npc_id.clone(),
            approach,
            effect: effect_of(dossier, approach),
            attitude,
            opposes: dossier.role.opposes_party(),
            reaction,
            emotional_state,
        });
        self.exchanges.send_replace(sequence);
        Recorded { shift }
    }

    /// Applies a deterministic threat before generating the NPC's answer. The caller saves after
    /// recording the completed exchange.
    pub(super) fn threaten(
        &self,
        member: MemberId,
        dossier: &Dossier,
        scene: &str,
    ) -> Option<Shift> {
        self.lock()
            .entry((member, dossier.npc_id.clone()))
            .or_insert_with(|| Relationship::new(dossier))
            .threaten(scene)
    }

    /// The NPC dossier for a scene person, by person id. Dossier ids are `role:person`; a
    /// caller may also pass the full dossier id or a longer canonical ref ending in the
    /// person id, so the owner of a witnessed theft is not missed for a spelling difference.
    fn dossier_for_person(&self, person_id: &str) -> Option<&Dossier> {
        let person = person_id.rsplit(':').next().unwrap_or(person_id);
        self.dossiers.iter().find(|dossier| {
            dossier.npc_id == person_id
                || dossier
                    .npc_id
                    .split_once(':')
                    .is_some_and(|(_, id)| id == person || id == person_id)
        })
    }

    /// Records one noticed theft against one hero for the owner and for a different noticer.
    /// The owner keeps the grievance and turns Hostile; the noticer keeps the grievance and
    /// cools one level (both through [`Relationship::record_theft`]). Idempotent per
    /// `operation_id` per NPC, so a replayed journal or a repeated NPC_EVENT reveal records
    /// nothing twice. Returns whether anything was new; the caller saves.
    pub(super) fn record_noticed_theft(
        &self,
        member: MemberId,
        owner: Option<&Dossier>,
        noticer: Option<&Dossier>,
        item_name: &str,
        operation_id: &str,
    ) -> bool {
        let mut recorded = false;
        if let Some(owner) = owner {
            let keeper = TheftKeeper::Owner {
                unforgiving: owner.emotional_profile().forgiveness == Forgiveness::Unforgiving,
            };
            recorded |= self
                .lock()
                .entry((member, owner.npc_id.clone()))
                .or_insert_with(|| Relationship::new(owner))
                .record_theft(operation_id, item_name, keeper);
        }
        if let Some(noticer) = noticer
            && owner.is_none_or(|owner| owner.npc_id != noticer.npc_id)
        {
            recorded |= self
                .lock()
                .entry((member, noticer.npc_id.clone()))
                .or_insert_with(|| Relationship::new(noticer))
                .record_theft(operation_id, item_name, TheftKeeper::Noticer);
        }
        recorded
    }

    /// Records owner consent that this hero may try on or handle `item_id`, given by the NPC
    /// whose dossier this is. Returns whether it was new; the caller saves.
    #[allow(dead_code)] // Wired by the dialogue/actor lane per M3 request.md; covered by tests.
    pub(super) fn record_try_on_consent(
        &self,
        member: MemberId,
        dossier: &Dossier,
        item_id: &str,
    ) -> bool {
        self.lock()
            .entry((member, dossier.npc_id.clone()))
            .or_insert_with(|| Relationship::new(dossier))
            .record_try_on_consent(item_id)
    }

    /// Whether this owner has consented for this hero to try on or handle `item_id`. Other
    /// code (for example a freeform handling judgement) reads this; it grants nothing itself.
    #[allow(dead_code)] // Read by handling/freeform code per M3 request.md; covered by tests.
    pub(super) fn has_try_on_consent(
        &self,
        member: MemberId,
        dossier: &Dossier,
        item_id: &str,
    ) -> bool {
        self.lock()
            .get(&(member, dossier.npc_id.clone()))
            .is_some_and(|relationship| {
                relationship
                    .try_on_consents
                    .iter()
                    .any(|consent| consent.item_id == item_id)
            })
    }

    /// The canonical item ids this hero may try on or handle with this NPC's consent.
    #[allow(dead_code)] // Read by handling/freeform code per M3 request.md; covered by tests.
    pub(super) fn try_on_consents(&self, member: MemberId, dossier: &Dossier) -> Vec<String> {
        self.lock()
            .get(&(member, dossier.npc_id.clone()))
            .map_or_else(Vec::new, |relationship| {
                relationship
                    .try_on_consents
                    .iter()
                    .map(|consent| consent.item_id.clone())
                    .collect()
            })
    }

    /// Records every witnessed theft in a replayed journal. The owner keeps the wrong and turns
    /// hostile to that hero; a different noticer keeps the wrong. Each attempt is recorded once per
    /// standing, so repeating the same journal changes nothing. Returns whether anything was new;
    /// the caller saves.
    pub(super) fn record_witnessed_thefts(
        &self,
        witnesses: &[SceneTheftWitness],
    ) -> Result<bool, SocialError> {
        let mut recorded = false;
        for witness in witnesses {
            let member =
                member_from_hex(&witness.member_id).ok_or(SocialError::Invalid("theft member"))?;
            let owner = witness
                .owner_id
                .as_deref()
                .and_then(|id| self.dossier_for_person(id));
            let noticer = self.dossier_for_person(&witness.noticer_id);
            recorded |= self.record_noticed_theft(
                member,
                owner,
                noticer,
                &witness.item_name,
                &witness.operation_id,
            );
        }
        Ok(recorded)
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
                    perceived_reaction: self
                        .lock()
                        .get(&(member, dossier.npc_id.clone()))
                        .map_or_else(
                            || EmotionalState::default().perceived_line(&actor.name, "you"),
                            |relationship| {
                                relationship.emotional.perceived_line(&actor.name, "you")
                            },
                        ),
                })
            })
            .collect();
        if let Some(room) = self.active_room() {
            for person in room
                .model
                .people
                .iter()
                .filter(|person| person.is_background)
            {
                let Some(dossier) = self.dossier_of(rpc::SceneRole::Bystander, &person.id) else {
                    continue;
                };
                if journey
                    .standing
                    .iter()
                    .any(|standing| standing.npc_id == dossier.npc_id)
                {
                    continue;
                }
                journey.standing.push(rpc::NpcStanding {
                    npc_id: dossier.npc_id.clone(),
                    name: person.name.clone(),
                    attitude: attitude_code(self.standing(member, dossier).attitude),
                    perceived_reaction: self
                        .lock()
                        .get(&(member, dossier.npc_id.clone()))
                        .map_or_else(
                            || EmotionalState::default().perceived_line(&person.name, "you"),
                            |relationship| {
                                relationship.emotional.perceived_line(&person.name, "you")
                            },
                        ),
                });
            }
        }
    }

    /// Records a hero's assault on this NPC: the NPC keeps it as a wrong in its own recollection,
    /// and its standing toward that hero becomes Hostile. The hero's line is condensed and bounded
    /// like any grievance. Returns the level shift when the standing changed.
    pub(super) fn record_assault(
        &self,
        member: MemberId,
        dossier: &Dossier,
        scene: &str,
        hero_line: &str,
    ) -> Option<Shift> {
        let words = hero_line
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(GRIEVANCE_CHARACTERS)
            .collect();
        let mut relationships = self.lock();
        let relationship = relationships
            .entry((member, dossier.npc_id.clone()))
            .or_insert_with(|| Relationship::new(dossier));
        relationship.begin_scene(scene);
        let from = relationship.attitude;
        // The attitude is set directly, so the grievance itself must not lower it a second time.
        relationship.record_grievance(
            Grievance {
                act: GrievanceAct::Wrong,
                words,
            },
            true,
        );
        relationship.attitude = Attitude::Hostile;
        (from != Attitude::Hostile).then_some(Shift::Level {
            from,
            to: Attitude::Hostile,
        })
    }

    /// Records a coerced demand the NPC was pressed with: a threat (Intimidation) or a lie it had to
    /// believe (Deception). The wrong is a lasting grievance against that hero only. A demand the NPC
    /// met turns it Hostile toward that hero; a refused demand keeps the grievance and lowers its
    /// standing one level, as any wrong does. Returns the level shift when the standing changed.
    pub(super) fn record_scene_threat(
        &self,
        member: MemberId,
        dossier: &Dossier,
        scene: &str,
        hero_line: &str,
        complied: bool,
    ) -> Option<Shift> {
        let words = hero_line
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(GRIEVANCE_CHARACTERS)
            .collect();
        let mut relationships = self.lock();
        let relationship = relationships
            .entry((member, dossier.npc_id.clone()))
            .or_insert_with(|| Relationship::new(dossier));
        relationship.begin_scene(scene);
        let from = relationship.attitude;
        relationship.record_grievance(
            Grievance {
                act: GrievanceAct::Threat,
                words,
            },
            complied,
        );
        if complied {
            relationship.attitude = Attitude::Hostile;
        }
        (from != relationship.attitude).then_some(Shift::Level {
            from,
            to: relationship.attitude,
        })
    }

    /// Records a planting this NPC noticed: the hero's wrong is kept as a grievance and lowers the
    /// standing one level, as a noticed wrong does. The hero's line is condensed and bounded.
    pub(super) fn record_noticed_plant(
        &self,
        member: MemberId,
        dossier: &Dossier,
        scene: &str,
        hero_line: &str,
    ) -> Option<Shift> {
        let words = hero_line
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(GRIEVANCE_CHARACTERS)
            .collect();
        let mut relationships = self.lock();
        let relationship = relationships
            .entry((member, dossier.npc_id.clone()))
            .or_insert_with(|| Relationship::new(dossier));
        relationship.begin_scene(scene);
        let from = relationship.attitude;
        relationship.record_grievance(
            Grievance {
                act: GrievanceAct::Wrong,
                words,
            },
            false,
        );
        (from != relationship.attitude).then_some(Shift::Level {
            from,
            to: relationship.attitude,
        })
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

pub(super) fn effect_of(dossier: &Dossier, approach: Approach) -> Effect {
    let profile = dossier.emotional_profile();
    let temperament = &profile.legacy_reactions;
    match approach {
        Approach::Flattery => temperament.flattery.effect,
        Approach::Flirting => temperament.flirting.effect,
        Approach::Apology => Effect::Warm,
        Approach::Introduction | Approach::Refusal => Effect::Steady,
        Approach::Intimidation => temperament.threats.effect,
        Approach::Deception => temperament.lies.effect,
        Approach::Bribe => temperament.bribes.effect,
        Approach::Kindness => Effect::Warm,
        Approach::Insult => Effect::Cool,
        Approach::Question | Approach::Persuasion | Approach::Bargain | Approach::Other => {
            Effect::Steady
        }
    }
}

/// The public table line for a shift, named for the NPC: "Marek cools toward Brakka."
pub(super) fn shift_note(npc: &str, shift: Shift, hero: &str) -> String {
    match shift {
        Shift::Level { from, to } if rank(to) > rank(from) => format!("{npc} warms to {hero}."),
        Shift::Level { .. } => format!("{npc} cools toward {hero}."),
        Shift::Trust { warmer: true } => format!("{npc} softens a little toward {hero}."),
        Shift::Trust { warmer: false } => format!("{npc} grows wary of {hero}."),
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
