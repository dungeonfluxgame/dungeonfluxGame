//! The production adapters of the engine's ports that have a real owner today: the `RulesOwner`
//! over `df-rules` and the `Dice` over a recorded entropy source (O.4, O.8).
//!
//! `df-table` stays pure and sees only the ports; this module is where `df-rules` is called. The
//! rules adapter (`rules`) runs ability and skill checks and the influence flow, with explicit
//! draws, for the heroes of a `SeatRoster` and the actions of an `ActionBook`; everything else it
//! declines `not_supported_yet` naming the missing piece (RES31). The dice (`dice`) draw from an
//! entropy source the host picks, seeded for tests and system-keyed for play, and record every
//! draw so a replay can supply the same dice again. `backend` reads the `rules_backend` setting
//! (`scripted`, the default, or `df-rules`) and builds the ports the dev engine starts with;
//! `dev_check` and `dev_route` are the dev verb `rules check`.
//!
//! Contract: the same tests run against the scripted fake of `df-table`'s testkit and against
//! this adapter wherever their behaviours overlap (`contract_tests`, O.5).

mod action_book;
pub mod backend;
pub mod dev_check;
pub mod dev_route;
mod dice;
mod error;
mod hero_profile;
mod rules;

pub use self::action_book::{
    ActionBook, DifficultyClass, InfluenceSetup, MissingPiece, RulesEntry, Runnable,
};
pub use self::backend::{DevPorts, RulesBackend, dev_ports, story_ports, story_urging};
pub use self::dev_check::{RulesCheckKit, RulesCheckReport, RulesCheckRequest};
pub use self::dice::{
    DrawRecord, EntropySource, MAXIMUM_RECORDED_DRAWS, RecordedDice, ReplayDice, SeededEntropy,
    SystemEntropy,
};
pub use self::error::AdapterError;
pub use self::hero_profile::{Ability, HeroProfile, SeatRoster, Skill, SkillBonus};
pub use self::rules::DfRulesOwner;

#[cfg(test)]
mod backend_tests;
#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod dev_check_tests;
#[cfg(test)]
mod dice_tests;
#[cfg(test)]
mod hero_profile_tests;
#[cfg(test)]
mod rules_tests;
#[cfg(test)]
mod wiring_tests;
