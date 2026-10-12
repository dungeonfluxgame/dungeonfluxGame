//! Pure source-pinned handler selection. Mechanics and source admission are separate owners.
mod command_handler;
mod dispatch;
mod explanation;

pub use command_handler::{InvocationError, RulesCommandHandler, RulesCommandInput, stage_handler};
pub use dispatch::{DispatchError, DispatchRegistry, HandlerRegistration, RegistryError};
pub use explanation::RuleExplanation;

pub mod ability_check;
pub mod current_responses;
pub mod preconditions;

pub mod local_journey;

mod srd_creatures;
pub use srd_creatures::{SRD_521_CREATURES, SrdCreature};

/// Source-pinned, pure grid combat subset; game integration is a separate milestone.
pub mod tactical;
