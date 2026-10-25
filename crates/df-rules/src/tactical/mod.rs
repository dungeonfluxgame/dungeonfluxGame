//! Pure SRD 5.2.1 tactical subset. See planning/tactical-combat.md for source clauses,
//! explicit geometry adjudication, unsupported modes and the durable integration handoff.
mod build;
mod development;
mod map;
mod resolve;
mod snapshot;
mod species;
mod state;
mod weapons;

pub use crate::local_journey::{
    ChallengeRating, HeroSpecies, OriginFeat, SpeciesCombatTraits, SpeciesSize,
    background_origin_feat, experience_points_for_challenge_rating,
};
pub use build::{ARCHER_CREATION_JSON, ArcherBuild, LEVEL_FIVE_BUILDS, validate_archer};
pub use development::DevelopmentEdit;
pub use map::{
    BattleMap, BattleSceneData, Cell, Cover, GridData, LayoutData, Lighting, ObstacleData,
    SpawnData,
};
pub use resolve::{
    Action, AttackReason, AttackSpend, AttackSummary, AttackUse, D20Reroll, D20Test,
    DeathSaveResult, DeathSaveSummary, DrawBinding, HelpPurpose, HelpSummary, MoveSummary,
    RecoverySummary, Resolution, SpeciesAction, StabilizationOutcome, StabilizationSummary,
    resolve, resolve_species,
};
pub use species::{BreathDirection, BreathShape, SavingThrowSummary, SpeciesReaction};
pub use state::{
    Battle, Budget, Combatant, CombatantSpec, CreatureExperience, CreatureKind,
    EncounterExperience, Health, PendingReaction, Side, Size, begin,
};
pub use weapons::{DamageType, Mastery, Weapon};

/// Local immutable rules/source revision, separate from the caller's canonical catalog pins.
pub const SOURCE_REVISION: &str = "SRD-5.2.1:PLAY-T01:3";
/// Fixed Fighter levels 1-5, Champion; pinned PDF pp.23,47-49,87.
pub const FIGHTER_SOURCE_REVISION: &str = "SRD-5.2.1:FIGHTER-1-5:1";
/// Shared attribution and PDF checksum are retained by the existing local journey authority.
pub const SOURCE: &str = crate::local_journey::SOURCE;
/// Explicit table adjudication: SRD Cover does not specify grid ray arithmetic.
pub const DEPARTURES: &[&str] = &[
    "tactical.cover-centre-line",
    "tactical.cover-representative-height",
    "tactical.initiative-ties-input-order",
    "tactical.dead-space-passable",
    "tactical.area-cell-centres-eight-directions",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuleError {
    Capacity,
    MapData,
    SnapshotData,
    DuplicateId,
    DuplicateCell,
    OutOfBounds,
    Spawn,
    UnreachableSpawn,
    UnknownLayout,
    InvalidCombatant,
    UnsupportedSize,
    UnsupportedCreature,
    InvalidBuild,
    UnknownActor,
    NotYourTurn,
    Incapacitated,
    ActionSpent,
    BonusActionSpent,
    ReactionSpent,
    MovementSpent,
    InvalidPath,
    Occupied,
    Blocked,
    NoPath,
    InvalidWeapon,
    InvalidHands,
    InvalidTarget,
    OutOfRange,
    TotalCover,
    AmmunitionMissing,
    WeaponMissing,
    InvalidExtraAttack,
    PendingReaction,
    NoPendingReaction,
    InvalidReaction,
    NeedDeathSave,
    InvalidDraw,
    MissingDraw { sides: u32 },
    UnusedDraws,
    StaleDraw,
    Finished,
    RecoveryUnavailable,
    HealersKitMissing,
    UnpricedExperience,
}

#[cfg(test)]
mod tests;
