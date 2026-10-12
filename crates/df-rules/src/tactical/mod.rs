//! Pure SRD 5.2.1 tactical subset. See planning/tactical-combat.md for source clauses,
//! explicit geometry adjudication, unsupported modes and the durable integration handoff.
mod build;
mod map;
mod resolve;
mod snapshot;
mod state;
mod weapons;

pub use build::{ARCHER_CREATION_JSON, ArcherBuild, validate_archer};
pub use map::{
    BattleMap, BattleSceneData, Cell, Cover, GridData, LayoutData, ObstacleData, SpawnData,
};
pub use resolve::{
    Action, AttackReason, AttackSpend, AttackSummary, AttackUse, DrawBinding, Resolution, resolve,
};
pub use state::{
    Battle, Budget, Combatant, CombatantSpec, CreatureKind, Health, Side, Size, begin,
};
pub use weapons::{DamageType, Mastery, Weapon};

/// Local immutable rules/source revision, separate from the caller's canonical catalog pins.
pub const SOURCE_REVISION: &str = "SRD-5.2.1:PLAY-T01:1";
/// Shared attribution and PDF checksum are retained by the existing local journey authority.
pub const SOURCE: &str = crate::local_journey::SOURCE;
/// Explicit table adjudication: SRD Cover does not specify grid ray arithmetic.
pub const DEPARTURES: &[&str] = &["tactical.cover-centre-line"];

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
}

#[cfg(test)]
mod tests;
