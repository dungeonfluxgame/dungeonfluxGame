//! Pure source-pinned handler selection. Mechanics and source admission are separate owners.
mod command_handler;
mod dispatch;
mod explanation;

pub use command_handler::{InvocationError, RulesCommandHandler, RulesCommandInput, stage_handler};
pub use dispatch::{DispatchError, DispatchRegistry, HandlerRegistration, RegistryError};
pub use explanation::RuleExplanation;

pub mod ability_check;
pub mod current_responses;
pub mod influence;
pub mod preconditions;

pub mod local_journey;
pub mod species;

mod srd_creatures;
pub use srd_creatures::{
    CreatureFeature, FeatureKind, FeatureStatus, SCENE_COMMONER, SRD_521_CREATURE_FEATURES,
    SRD_521_CREATURES, SrdCreature, missing_features,
};

/// Source-pinned, pure grid combat subset; game integration is a separate milestone.
pub mod coverage;
pub mod tactical;
