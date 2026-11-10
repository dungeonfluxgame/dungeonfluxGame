//! The music table: everything about how a journey sounds, as data.
//!
//! One JSON file holds the engine's policy (the numbers `df_tempo::presentation` runs on), the
//! impulse kinds a journey posts, the director's rules and the profiles (a closed `when`
//! condition on the tempo, the composer's prompt, the length, the volume, the crossfade). Nothing
//! in the host names a story, a place or a style. A table that does not read or does not hold
//! together is a typed refusal and changes nothing.
//!
//! Profiles are tried in table order and the first whose `when` holds is the one the tempo wants;
//! `base` names the profile that plays at the start and when none holds.

use std::collections::HashSet;

use df_playground::limits::MAXIMUM_VOLUME_PERCENT;
use df_playground::scene::Target;
use df_tempo::presentation::PresentationPolicy;
use serde::{Deserialize, Serialize};

use super::limits::{
    MAXIMUM_CROSSFADE_MS, MAXIMUM_DWELL_MS, MAXIMUM_ID_CHARACTERS, MAXIMUM_IMPULSE_KINDS,
    MAXIMUM_PERMILLE, MAXIMUM_PROFILES, MAXIMUM_WINDOW_MS,
};
use super::request::{ConditionCause, Momentum, PhaseKey, TableCause, TempoError};
use crate::dev_http::parse_body;
use crate::imagegen::model::plain_name;
use crate::music::MusicRequest;
use crate::music::limits::{
    MAXIMUM_LENGTH_SECONDS, MAXIMUM_PROMPT_CHARACTERS, MINIMUM_LENGTH_SECONDS,
};

/// The music table file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MusicTable {
    /// The table's version; the policy version the views report.
    pub version: u32,
    pub policy: PolicyData,
    pub director: DirectorData,
    pub impulses: Vec<ImpulseRule>,
    /// The profile that plays at the start and when no `when` holds.
    pub base: String,
    pub profiles: Vec<Profile>,
}

/// The engine's policy in the units of the table file (milliseconds and permille).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyData {
    pub inertia_half_life_ms: u32,
    pub decay_half_life_ms: u32,
    pub fatigue_half_life_ms: u32,
    pub fatigue_charge_permille: u16,
    pub fatigue_attenuation_permille: u16,
    /// Where calm gives way to build, build to anticipation, anticipation to peak.
    pub thresholds: [u16; 3],
    pub hysteresis_permille: u16,
    /// The intensity the journey starts at and rests at.
    pub rest_permille: u16,
    /// Inertia within this far of zero is a stable momentum.
    pub momentum_deadband_permille: u16,
}

impl PolicyData {
    pub(super) fn presentation(self) -> PresentationPolicy {
        PresentationPolicy {
            inertia_half_life_millis: self.inertia_half_life_ms,
            decay_half_life_millis: self.decay_half_life_ms,
            fatigue_half_life_millis: self.fatigue_half_life_ms,
            fatigue_charge_permille: self.fatigue_charge_permille,
            fatigue_attenuation_permille: self.fatigue_attenuation_permille,
            thresholds: self.thresholds,
            hysteresis_permille: self.hysteresis_permille,
        }
    }
}

/// How the director changes tracks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectorData {
    /// How long a cue plays before another may replace it.
    pub min_dwell_ms: u64,
    /// How often the director decides when no impulse comes: the tempo settles and the recent
    /// impulses expire with the time, and nothing else would tell the director.
    pub tick_ms: u64,
    /// How far below its lower bound (or above its upper) the playing profile's intensity may
    /// drift before it counts as no longer applying, unless a profile before it applies.
    pub hold_margin_permille: u16,
    /// What every track's library name starts with.
    pub track_prefix: String,
    /// The screens the music plays on.
    pub target: Target,
}

/// What one kind of event does to the tempo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImpulseRule {
    pub kind: String,
    /// The push on the intensity at full strength, in permille (-1000 to 1000).
    pub impulse_permille: i16,
    /// The intensity the tempo settles toward after it.
    pub target_permille: u16,
    /// A release: the push cannot lift the intensity above the new target's level and the
    /// engine classifies the phase as a release.
    #[serde(default)]
    pub release: bool,
}

/// When a profile applies: every field that is present must hold.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Condition {
    /// The engine's phase is one of these.
    pub phases: Option<Vec<PhaseKey>>,
    pub intensity_min: Option<u16>,
    pub intensity_max: Option<u16>,
    pub momentum: Option<Momentum>,
    /// One of these kinds of impulse happened within the window.
    pub recent: Option<RecentCondition>,
}

/// An impulse of one of `kinds` within the last `within_ms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecentCondition {
    pub kinds: Vec<String>,
    pub within_ms: u64,
}

/// One score the director can play.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    #[serde(default)]
    pub when: Condition,
    /// What the composer is asked for; the whole style lives here.
    pub prompt: String,
    pub length_seconds: u32,
    pub volume_percent: u8,
    #[serde(rename = "loop", default = "loops")]
    pub looping: bool,
    /// The time a change into this profile takes, in milliseconds: the old track dips out, the
    /// new one dips in, half of it each.
    pub crossfade_ms: u32,
}

fn loops() -> bool {
    true
}

impl MusicTable {
    /// Reads and checks a table from its JSON.
    pub fn from_json(bytes: &[u8]) -> Result<Self, TempoError> {
        let table: Self = parse_body(bytes).map_err(TempoError::InvalidBody)?;
        table
            .check()
            .map_err(|cause| TempoError::InvalidTable { cause })?;
        Ok(table)
    }

    /// That the table holds together: bounds, unique ids, a base that exists, conditions that
    /// name kinds the table has.
    pub fn check(&self) -> Result<(), TableCause> {
        if self.profiles.is_empty() {
            return Err(TableCause::NoProfiles);
        }
        if self.impulses.is_empty() {
            return Err(TableCause::NoImpulses);
        }
        if self.profiles.len() > MAXIMUM_PROFILES || self.impulses.len() > MAXIMUM_IMPULSE_KINDS {
            return Err(TableCause::TooMany);
        }
        self.policy
            .presentation()
            .validate()
            .map_err(|_| TableCause::InvalidPolicy)?;
        if self.policy.rest_permille > MAXIMUM_PERMILLE
            || self.policy.momentum_deadband_permille > MAXIMUM_PERMILLE
        {
            return Err(TableCause::InvalidPolicy);
        }
        self.check_director()?;
        self.check_impulses()?;
        self.check_profiles()?;
        if self.profile(&self.base).is_none() {
            return Err(TableCause::UnknownBase {
                id: self.base.clone(),
            });
        }
        Ok(())
    }

    fn check_director(&self) -> Result<(), TableCause> {
        let director = &self.director;
        if director.min_dwell_ms > MAXIMUM_DWELL_MS
            || director.tick_ms == 0
            || director.tick_ms > MAXIMUM_DWELL_MS
            || director.hold_margin_permille > MAXIMUM_PERMILLE
            || !plain_name(&director.track_prefix)
        {
            return Err(TableCause::InvalidDirector);
        }
        Ok(())
    }

    fn check_impulses(&self) -> Result<(), TableCause> {
        let mut seen = HashSet::new();
        for rule in &self.impulses {
            if !is_id(&rule.kind) {
                return Err(TableCause::InvalidId {
                    id: rule.kind.clone(),
                });
            }
            if !seen.insert(rule.kind.as_str()) {
                return Err(TableCause::DuplicateImpulse {
                    kind: rule.kind.clone(),
                });
            }
            if !(-1_000..=1_000).contains(&rule.impulse_permille)
                || rule.target_permille > MAXIMUM_PERMILLE
            {
                return Err(TableCause::InvalidImpulse {
                    kind: rule.kind.clone(),
                });
            }
        }
        Ok(())
    }

    fn check_profiles(&self) -> Result<(), TableCause> {
        let mut seen = HashSet::new();
        for profile in &self.profiles {
            if !is_id(&profile.id) || !plain_name(&profile.track_name(&self.director)) {
                return Err(TableCause::InvalidId {
                    id: profile.id.clone(),
                });
            }
            if !seen.insert(profile.id.as_str()) {
                return Err(TableCause::DuplicateProfile {
                    id: profile.id.clone(),
                });
            }
            profile.check_bounds()?;
            profile.check_condition(self)?;
        }
        Ok(())
    }

    /// The impulse rule of `kind`.
    pub fn impulse(&self, kind: &str) -> Option<&ImpulseRule> {
        self.impulses.iter().find(|rule| rule.kind == kind)
    }

    /// The profile named `id`.
    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    /// The place of the base profile in the table. A checked table has one.
    pub(super) fn base_index(&self) -> usize {
        self.profiles
            .iter()
            .position(|profile| profile.id == self.base)
            .unwrap_or_default()
    }
}

impl Profile {
    fn check_bounds(&self) -> Result<(), TableCause> {
        let prompt_characters = self.prompt.chars().count();
        if self.prompt.trim().is_empty() || prompt_characters > MAXIMUM_PROMPT_CHARACTERS {
            return Err(TableCause::InvalidPrompt {
                id: self.id.clone(),
                prompt_characters,
            });
        }
        if !(MINIMUM_LENGTH_SECONDS..=MAXIMUM_LENGTH_SECONDS).contains(&self.length_seconds) {
            return Err(TableCause::InvalidLength {
                id: self.id.clone(),
                length_seconds: self.length_seconds,
            });
        }
        if self.volume_percent > MAXIMUM_VOLUME_PERCENT {
            return Err(TableCause::InvalidVolume {
                id: self.id.clone(),
                volume_percent: self.volume_percent,
            });
        }
        if self.crossfade_ms > MAXIMUM_CROSSFADE_MS {
            return Err(TableCause::InvalidCrossfade {
                id: self.id.clone(),
                crossfade_ms: self.crossfade_ms,
            });
        }
        Ok(())
    }

    fn check_condition(&self, table: &MusicTable) -> Result<(), TableCause> {
        let refused = |condition| TableCause::InvalidCondition {
            id: self.id.clone(),
            condition,
        };
        let when = &self.when;
        if when.phases.as_ref().is_some_and(Vec::is_empty) {
            return Err(refused(ConditionCause::EmptyPhases));
        }
        let (low, high) = (when.intensity_min, when.intensity_max);
        if low.is_some_and(|bound| bound > MAXIMUM_PERMILLE)
            || high.is_some_and(|bound| bound > MAXIMUM_PERMILLE)
            || low.zip(high).is_some_and(|(low, high)| low > high)
        {
            return Err(refused(ConditionCause::InvalidIntensityRange));
        }
        if let Some(recent) = &when.recent {
            if recent.kinds.is_empty()
                || recent.within_ms == 0
                || recent.within_ms > MAXIMUM_WINDOW_MS
            {
                return Err(refused(ConditionCause::InvalidRecent));
            }
            if recent
                .kinds
                .iter()
                .any(|kind| table.impulse(kind).is_none())
            {
                return Err(refused(ConditionCause::UnknownImpulseKind));
            }
        }
        Ok(())
    }

    /// The library name of this profile's track: the prefix, the id and a hash of what the
    /// composer is asked for, so a changed prompt or length is a new track and an unchanged one
    /// is found again.
    pub fn track_name(&self, director: &DirectorData) -> String {
        let hash = style_hash(&self.prompt, self.length_seconds);
        format!("{}-{}-{hash:08x}", director.track_prefix, self.id)
    }

    /// The request that makes or finds this profile's track.
    pub fn music_request(&self, director: &DirectorData) -> MusicRequest {
        MusicRequest {
            name: self.track_name(director),
            prompt: self.prompt.clone(),
            length_seconds: self.length_seconds,
            target: director.target,
            volume_percent: self.volume_percent,
        }
    }
}

/// A profile or impulse id: letters, digits, '-' and '_', at most `MAXIMUM_ID_CHARACTERS`.
pub(super) fn is_id(id: &str) -> bool {
    id.chars().count() <= MAXIMUM_ID_CHARACTERS && plain_name(id)
}

/// FNV-1a over the prompt and the length, folded to 32 bits. Stable across builds, which a track
/// name kept in the library needs.
fn style_hash(prompt: &str, length_seconds: u32) -> u32 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let bytes = prompt.bytes().chain(length_seconds.to_le_bytes());
    let hash = bytes.fold(OFFSET, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(PRIME)
    });
    let folded = (hash >> 32) ^ (hash & u64::from(u32::MAX));
    // Both halves are under 2^32, so the fold always fits.
    u32::try_from(folded).unwrap_or(u32::MAX)
}
