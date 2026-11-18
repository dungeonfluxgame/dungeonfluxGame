//! The creation machine on the dev host (`planning/phone-and-creation.md` P2, P3): each seated
//! seat builds its hero from the species and class its look-studio hero was started with, the
//! rules (`df_rules::hero`) derive every number, the seat's Ready in the lobby is its submitted
//! build, and starting play hands one sheet per hero on, in the shape the check judge reads.
//!
//! Reading order: `rules` (the machine's rules port over `df-rules`, and the check profile of a
//! sheet), `request` (the typed refusal and the dev routes' values), `store` (the saved record),
//! `view` (what a phone is shown), `desk` (the steps and the `CREATION` lines), `vitals` (a built
//! hero's live state), `cast` (a hero casting a spell), `routes` (a seat's routes on both
//! listeners and the loopback dev routes), `format` and `cli` (the `creation` verbs). Every step
//! writes `CREATION` lines: `transition`, `derived`, `started`, `refused`. A line names seats, hero
//! ids, states, slots and counts, never a name.
mod cast;
pub mod cli;
mod desk;
mod format;
pub mod request;
pub mod routes;
pub mod rules;
mod store;
mod view;
mod vitals;

pub(crate) use self::cast::cast_spell;
pub use self::cast::{CastError, CastOutcome};
pub use self::cli::{CREATION_USAGE, CreationCliError, CreationCommand, parse, run};
pub use self::desk::{HeroSheetSummary, hero_summary};
pub(crate) use self::desk::{
    hero_sheet, ready_waits, release_seat, reopen_after_unready, seat_error, sheet_file, start_play,
};
pub use self::rules::{HeroRules, hero_profile};
pub use self::view::sheet_view;
pub(crate) use self::vitals::{KitOutcome, give_kit_again};
pub use self::vitals::{apply_vitals, hero_gear, hero_vitals};

#[cfg(test)]
mod cast_tests;
#[cfg(test)]
mod creation_tests;
#[cfg(test)]
mod vitals_tests;
