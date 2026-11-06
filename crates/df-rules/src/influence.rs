//! The Influence action (SRD 5.2.1, Rules Glossary "Influence [Action]", printed p. 184).
//!
//! A hero urges a monster to do something. The caller speaks for the GM side and
//! decides whether the monster feels willing, hesitant, or unwilling about what
//! is asked; only a hesitant monster calls for a check. The check is affected by
//! the monster's attitude (Friendly grants Advantage, p. 182; Hostile imposes
//! Disadvantage, p. 183) and uses a default DC of 15 or the monster's
//! Intelligence score, whichever is higher (p. 184). On a failed check the same
//! urging must wait 24 hours, or a duration the GM sets, before it is tried
//! again (p. 184).
//!
//! The quality of the player's words is one more Advantage/Disadvantage source.
//! The source lets the GM grant Advantage for circumstances (p. 8) and says
//! roleplaying efforts can alter an NPC's attitude (p. 11), but the only stated
//! Influence-check modifiers are the attitude ones, so the standing
//! word-quality rule is disclosed as the house rule [`WORD_QUALITY_RULE_ID`],
//! following the same `DEPARTURES` pattern as `tactical.cover-centre-line`.
//! This crate never judges words: the caller supplies the tilt and a short
//! reason. Heroic Inspiration (expend to reroll one die and use the new roll,
//! pp. 8 and 183) is a separate reroll mechanic, not Advantage, and is not
//! modelled here.
//!
//! The authority supplies every die face, so results are reproducible. Nothing
//! here draws randomness or reads a clock.

/// Pinned source for this rule: SRD 5.2.1 English, Influence glossary entry.
pub const SOURCE_REVISION: &str = "SRD-5.2.1:page-184:Influence-Action";
/// Shared attribution; the PDF checksum matches the rules-sources manifest.
pub const SOURCE: &str = crate::local_journey::SOURCE;
/// Disclosed standing custom rule: player word quality grants Advantage when
/// favourable and Disadvantage when unfavourable, with a caller-supplied reason.
pub const WORD_QUALITY_RULE_ID: &str = "influence.word-quality-advantage";
/// Custom rules this module applies, in the form the project uses for disclosed
/// custom rules (`tactical.cover-centre-line` is declared the same way).
pub const DEPARTURES: &[&str] = &[WORD_QUALITY_RULE_ID];
/// Default Influence DC before the monster's Intelligence score is considered.
pub const INFLUENCE_DEFAULT_DC: u8 = 15;
/// Standard wait after a failed check before urging the same thing the same way.
pub const RETRY_WAIT_HOURS: u32 = 24;

/// Monster attitude toward the hero: Friendly, Indifferent, or Hostile.
/// Indifferent is the default attitude of a monster (p. 184).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Attitude {
    Friendly,
    Indifferent,
    Hostile,
}

impl Attitude {
    /// Table-facing word for this attitude.
    pub fn word(self) -> &'static str {
        match self {
            Self::Friendly => "friendly",
            Self::Indifferent => "indifferent",
            Self::Hostile => "hostile",
        }
    }
}

/// The five checks the source allows for Influence (Influence Checks table,
/// p. 184). Four are Charisma checks; Animal Handling is a Wisdom check usable
/// only to gently coax a Beast or Monstrosity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InfluenceSkill {
    Deception,
    Intimidation,
    Performance,
    Persuasion,
    AnimalHandling,
}

impl InfluenceSkill {
    /// Table-facing skill name, as the source writes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Deception => "Deception",
            Self::Intimidation => "Intimidation",
            Self::Performance => "Performance",
            Self::Persuasion => "Persuasion",
            Self::AnimalHandling => "Animal Handling",
        }
    }

    /// Ability the check uses: Charisma for the four social skills (Action
    /// Summary, p. 10), Wisdom for Animal Handling (Influence Checks, p. 184).
    pub fn ability(self) -> &'static str {
        match self {
            Self::AnimalHandling => "Wisdom",
            _ => "Charisma",
        }
    }
}

/// How the monster feels about what is asked, as the GM determines (p. 184).
/// Willing and unwilling monsters never roll: the monster fulfills a welcome
/// request as it prefers, and does not comply with a repugnant one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Willingness {
    Willing,
    Hesitant,
    Unwilling,
}

/// Why no roll was made.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoRollCause {
    /// Aligned with the monster's desires; it fulfills the request as it prefers.
    Willing,
    /// Repugnant to the monster or counter to its alignment; it does not comply.
    Unwilling,
}

/// The caller's read of the player's words, with a required short reason the
/// table can show (for example "you named what he is proud of"). This crate
/// never judges words; the conversation code supplies both fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WordQuality<'a> {
    pub tilt: WordTilt,
    pub reason: &'a str,
}

/// Whether the player's words help, hinder, or neither.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WordTilt {
    Favourable,
    Neutral,
    Unfavourable,
}

/// One Advantage or Disadvantage source with the short label the table reads.
/// Labels are owned so a resolved outcome never borrows the caller's input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModeSource {
    pub advantage: bool,
    pub label: String,
}

/// Combined roll mode. Several sources on one side never stack: two d20s are
/// still rolled (p. 8).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RollMode {
    Normal,
    Advantage,
    Disadvantage,
}

impl RollMode {
    /// Table-facing word for this mode.
    pub fn word(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Advantage => "Advantage",
            Self::Disadvantage => "Disadvantage",
        }
    }
}

/// The source's own attitude-based Advantage or Disadvantage, if it states one:
/// Advantage for a Friendly creature (p. 182), Disadvantage for a Hostile one
/// (p. 183), none for Indifferent. Applied automatically by [`resolve`].
pub fn attitude_source(attitude: Attitude) -> Option<ModeSource> {
    match attitude {
        Attitude::Friendly => Some(ModeSource {
            advantage: true,
            label: "Friendly toward you".to_string(),
        }),
        Attitude::Indifferent => None,
        Attitude::Hostile => Some(ModeSource {
            advantage: false,
            label: "Hostile toward you".to_string(),
        }),
    }
}

/// The owner's word-quality source: favourable words grant Advantage and
/// unfavourable words impose Disadvantage under house rule
/// [`WORD_QUALITY_RULE_ID`]; neutral words contribute nothing. The label is the
/// caller's reason, unchanged.
pub fn word_source(words: &WordQuality<'_>) -> Option<ModeSource> {
    match words.tilt {
        WordTilt::Favourable => Some(ModeSource {
            advantage: true,
            label: words.reason.to_string(),
        }),
        WordTilt::Neutral => None,
        WordTilt::Unfavourable => Some(ModeSource {
            advantage: false,
            label: words.reason.to_string(),
        }),
    }
}

/// Combine any number of Advantage and Disadvantage sources by the source's
/// rule (p. 8): sources on one side alone set that mode and stay listed as the
/// reasons that applied; both sides together cancel to Normal whatever their
/// counts, and then no reason applied. Returns the mode and the applied reasons.
pub fn combine_sources(sources: &[ModeSource]) -> (RollMode, Vec<ModeSource>) {
    let mut advantage = Vec::new();
    let mut disadvantage = Vec::new();
    for source in sources {
        if source.advantage {
            advantage.push(source.clone());
        } else {
            disadvantage.push(source.clone());
        }
    }
    if !advantage.is_empty() && disadvantage.is_empty() {
        (RollMode::Advantage, advantage)
    } else if advantage.is_empty() && !disadvantage.is_empty() {
        (RollMode::Disadvantage, disadvantage)
    } else {
        (RollMode::Normal, Vec::new())
    }
}

/// Standard Influence roll mode, including Help or other admitted circumstances. Sources on
/// opposite sides cancel and multiple sources on one side never stack (SRD 5.2.1 p. 8).
/// Word quality is deliberately absent: the standard path changes attitude, never adds a bonus.
pub fn roll_mode(attitude: Attitude, extra_sources: &[ModeSource]) -> RollMode {
    let mut sources = Vec::with_capacity(extra_sources.len() + 1);
    sources.extend(attitude_source(attitude));
    sources.extend_from_slice(extra_sources);
    combine_sources(&sources).0
}

/// Default DC rule (p. 184): 15 or the monster's Intelligence score, whichever
/// is higher. The GM chooses which check is made; the score (not its modifier)
/// sets the floor above 15.
pub fn difficulty_class(monster_intelligence: u8) -> u8 {
    monster_intelligence.max(INFLUENCE_DEFAULT_DC)
}

/// Everything [`resolve`] needs. Dice are supplied faces, never drawn here:
/// one face for a Normal roll, two for Advantage or Disadvantage.
pub struct InfluenceInput<'a> {
    pub ability_modifier: i8,
    pub proficiency_bonus: u8,
    pub skill: InfluenceSkill,
    pub attitude: Attitude,
    pub monster_intelligence: u8,
    pub willingness: Willingness,
    pub words: WordQuality<'a>,
    pub extra_sources: &'a [ModeSource],
    pub faces: &'a [u8],
}

/// What went wrong before any roll. Bounds mirror `ability_check::resolve`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InfluenceError {
    InvalidDie,
    InvalidAbilityModifier,
    UnsupportedProficiencyBonus,
    InvalidIntelligence,
    MissingWordReason,
    WrongFaceCount,
}

/// Checked outcome of one Influence action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InfluenceOutcome {
    Success,
    Failure,
    NoRoll(NoRollCause),
}

/// Resolved Influence action: outcome, the d20 face or faces, which face
/// counted, the total, the DC, and what follows a failure.
#[derive(Clone, Debug, PartialEq)]
pub struct InfluenceResult {
    pub skill: InfluenceSkill,
    pub mode: RollMode,
    /// Reasons that applied to the roll; empty when Normal.
    pub applied: Vec<ModeSource>,
    /// Every admitted source, retained even when Advantage and Disadvantage cancel.
    pub sources: Vec<ModeSource>,
    pub faces: Vec<u8>,
    /// Face that counted (highest with Advantage, lowest with Disadvantage);
    /// `None` when no roll was made.
    pub counted: Option<u8>,
    pub ability_modifier: i8,
    pub proficiency_bonus: u8,
    pub total: Option<i16>,
    pub difficulty_class: u8,
    pub outcome: InfluenceOutcome,
    /// Hours to wait before urging the same thing the same way again after a
    /// failure: always 24 here; the GM may set another duration (p. 184).
    pub retry_same_request_hours: Option<u32>,
}

impl InfluenceResult {
    /// Table-ready sentence in the illustrated shape, for example
    /// "Persuasion, Advantage (Friendly toward you; you named what he is proud
    /// of): 14 and 6, 14 + 2 = 16 against 15: success".
    pub fn explanation(&self) -> String {
        match self.outcome {
            InfluenceOutcome::NoRoll(NoRollCause::Willing) => {
                "No roll: willing, no check is necessary; the monster fulfills your request as it \
                    prefers."
                    .to_string()
            }
            InfluenceOutcome::NoRoll(NoRollCause::Unwilling) => {
                "No roll: unwilling, no check is necessary; the monster does not comply."
                    .to_string()
            }
            InfluenceOutcome::Success | InfluenceOutcome::Failure => {
                let faces = self
                    .faces
                    .iter()
                    .map(|face| face.to_string())
                    .collect::<Vec<_>>()
                    .join(" and ");
                let mut head = format!("{}, {}", self.skill.name(), self.mode.word());
                if !self.applied.is_empty() {
                    let reasons = self
                        .applied
                        .iter()
                        .map(|source| source.label.as_str())
                        .collect::<Vec<_>>()
                        .join("; ");
                    head.push_str(&format!(" ({reasons})"));
                }
                let counted = self.counted.unwrap_or(0);
                let modifier = i16::from(self.ability_modifier) + i16::from(self.proficiency_bonus);
                let arithmetic = if modifier >= 0 {
                    format!("{counted} + {modifier} = {}", self.total.unwrap_or(0))
                } else {
                    format!(
                        "{counted} - {} = {}",
                        modifier.abs(),
                        self.total.unwrap_or(0)
                    )
                };
                let verdict = if self.outcome == InfluenceOutcome::Success {
                    "success"
                } else {
                    "failure"
                };
                format!(
                    "{head}: {faces}, {arithmetic} against {}: {verdict}",
                    self.difficulty_class
                )
            }
        }
    }
}

/// Resolve one Influence action without I/O, randomness, or clock reads.
/// A proficiency bonus of zero means the hero lacks that skill's proficiency.
/// Willing and unwilling monsters return [`InfluenceOutcome::NoRoll`] without
/// validating the check inputs and without touching the supplied faces.
pub fn resolve(input: InfluenceInput<'_>) -> Result<InfluenceResult, InfluenceError> {
    let difficulty_class = difficulty_class(input.monster_intelligence);
    match input.willingness {
        Willingness::Willing => return Ok(no_roll(&input, difficulty_class, NoRollCause::Willing)),
        Willingness::Unwilling => {
            return Ok(no_roll(&input, difficulty_class, NoRollCause::Unwilling));
        }
        Willingness::Hesitant => {}
    }
    if !(1..=30).contains(&input.monster_intelligence) {
        return Err(InfluenceError::InvalidIntelligence);
    }
    if !(-5..=10).contains(&input.ability_modifier) {
        return Err(InfluenceError::InvalidAbilityModifier);
    }
    if input.proficiency_bonus != 0 && !(2..=6).contains(&input.proficiency_bonus) {
        return Err(InfluenceError::UnsupportedProficiencyBonus);
    }
    if input.words.reason.trim().is_empty() {
        return Err(InfluenceError::MissingWordReason);
    }
    let mut sources = Vec::new();
    if let Some(source) = attitude_source(input.attitude) {
        sources.push(source);
    }
    if let Some(source) = word_source(&input.words) {
        sources.push(source);
    }
    sources.extend(input.extra_sources.iter().cloned());
    let (mode, applied) = combine_sources(&sources);
    let expected_faces = match mode {
        RollMode::Normal => 1,
        RollMode::Advantage | RollMode::Disadvantage => 2,
    };
    if input.faces.len() != expected_faces {
        return Err(InfluenceError::WrongFaceCount);
    }
    for face in input.faces {
        if !(1..=20).contains(face) {
            return Err(InfluenceError::InvalidDie);
        }
    }
    let counted = match mode {
        RollMode::Normal => input.faces[0],
        RollMode::Advantage => *input.faces.iter().max().unwrap_or(&input.faces[0]),
        RollMode::Disadvantage => *input.faces.iter().min().unwrap_or(&input.faces[0]),
    };
    let total =
        i16::from(counted) + i16::from(input.ability_modifier) + i16::from(input.proficiency_bonus);
    let succeeded = total >= i16::from(difficulty_class);
    Ok(InfluenceResult {
        skill: input.skill,
        mode,
        applied,
        sources,
        faces: input.faces.to_vec(),
        counted: Some(counted),
        ability_modifier: input.ability_modifier,
        proficiency_bonus: input.proficiency_bonus,
        total: Some(total),
        difficulty_class,
        outcome: if succeeded {
            InfluenceOutcome::Success
        } else {
            InfluenceOutcome::Failure
        },
        retry_same_request_hours: if succeeded {
            None
        } else {
            Some(RETRY_WAIT_HOURS)
        },
    })
}

/// The result for a monster whose willingness settles the request without a check.
fn no_roll(
    input: &InfluenceInput<'_>,
    difficulty_class: u8,
    cause: NoRollCause,
) -> InfluenceResult {
    InfluenceResult {
        skill: input.skill,
        mode: RollMode::Normal,
        applied: Vec::new(),
        sources: Vec::new(),
        faces: Vec::new(),
        counted: None,
        ability_modifier: input.ability_modifier,
        proficiency_bonus: input.proficiency_bonus,
        total: None,
        difficulty_class,
        outcome: InfluenceOutcome::NoRoll(cause),
        retry_same_request_hours: None,
    }
}

/// A social approach the conversation code can name, mapped to the skill the
/// source would use. Where the source does not decide, `source_decided` is
/// false and the entry names the nearest skill with its reason; the GM still
/// chooses the check (p. 184).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Approach {
    Flatter,
    Charm,
    Seduce,
    Plead,
    Bargain,
    Reason,
    Deceive,
    Threaten,
    Provoke,
    Perform,
}

/// One approach row: its skill, whether the source decides it, and why.
pub struct ApproachSkill {
    pub approach: Approach,
    pub skill: InfluenceSkill,
    pub source_decided: bool,
    /// Page-grounded reason; entries the source does not decide say so.
    pub reason: &'static str,
}

/// Approach-to-skill table for the conversation code. Source-decided rows come
/// from the Influence Checks table (p. 184); skill descriptions come from the
/// Skills table (p. 9). Animal Handling has no row: the source assigns it only
/// to gently coaxing a Beast or Monstrosity (p. 184).
pub const APPROACH_SKILLS: &[ApproachSkill] = &[
    ApproachSkill {
        approach: Approach::Flatter,
        skill: InfluenceSkill::Persuasion,
        source_decided: false,
        reason: "SRD p9: Persuasion honestly and graciously convinces; flattery courts favour. The \
            source does not assign flattery; the GM chooses the check (p184).",
    },
    ApproachSkill {
        approach: Approach::Charm,
        skill: InfluenceSkill::Persuasion,
        source_decided: false,
        reason: "SRD p9: winning someone over graciously is nearest to Persuasion. The source does \
            not assign charm; the GM chooses the check (p184).",
    },
    ApproachSkill {
        approach: Approach::Seduce,
        skill: InfluenceSkill::Persuasion,
        source_decided: false,
        reason: "SRD p9: winning someone over is nearest to Persuasion; if the seduction is a lie \
            the GM may call Deception instead. The source does not decide (p184).",
    },
    ApproachSkill {
        approach: Approach::Plead,
        skill: InfluenceSkill::Persuasion,
        source_decided: false,
        reason: "SRD p184: gently persuading is Persuasion; a plea is nearest to it. The source \
            does not assign pleas; the GM chooses the check.",
    },
    ApproachSkill {
        approach: Approach::Bargain,
        skill: InfluenceSkill::Persuasion,
        source_decided: false,
        reason: "SRD p9: negotiating agreement honestly is nearest to Persuasion. The source does \
            not assign bargaining; the GM chooses the check (p184).",
    },
    ApproachSkill {
        approach: Approach::Reason,
        skill: InfluenceSkill::Persuasion,
        source_decided: false,
        reason: "SRD p9: convincing through honest argument is nearest to Persuasion. The source \
            does not assign reasoning; the GM chooses the check (p184).",
    },
    ApproachSkill {
        approach: Approach::Deceive,
        skill: InfluenceSkill::Deception,
        source_decided: true,
        reason: "SRD p184: Deceiving a monster that understands you is Charisma (Deception).",
    },
    ApproachSkill {
        approach: Approach::Threaten,
        skill: InfluenceSkill::Intimidation,
        source_decided: true,
        reason: "SRD p184: Intimidating a monster is Charisma (Intimidation).",
    },
    ApproachSkill {
        approach: Approach::Provoke,
        skill: InfluenceSkill::Intimidation,
        source_decided: false,
        reason: "SRD p9: goading through menace is nearest to aweing or threatening \
            (Intimidation). The source does not assign provoking; the GM chooses the check (p184).",
    },
    ApproachSkill {
        approach: Approach::Perform,
        skill: InfluenceSkill::Performance,
        source_decided: true,
        reason: "SRD p184: Amusing a monster is Charisma (Performance).",
    },
];

/// Look up the skill for one approach.
pub fn skill_for_approach(approach: Approach) -> InfluenceSkill {
    APPROACH_SKILLS
        .iter()
        .find(|entry| entry.approach == approach)
        .map(|entry| entry.skill)
        .unwrap_or(InfluenceSkill::Persuasion)
}

#[cfg(test)]
mod influence_tests;
