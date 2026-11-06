//! `media generate` as a command of the dev command line. Its grammar, usage line and sending are
//! in `imagegen::cli`, the way `cast::cli` holds the grammar of `scene cue`; this module only joins
//! it to `Command`.
//!
//! Transitional: once `command.rs`, `command/playground.rs` and `command/usage.rs` name
//! `crate::imagegen::cli` directly, this module goes.

pub(super) use crate::imagegen::cli::MEDIA_GENERATE_USAGE;
pub use crate::imagegen::cli::MediaGenerate;

use super::{Command, CommandError};

pub(super) fn parse(words: &[String]) -> Result<Command, CommandError> {
    crate::imagegen::cli::parse(words).map(Command::MediaGenerate)
}
