//! The one place a child process is built. A child inherits nothing from the runner: the
//! environment is cleared, so provider keys, database credentials and `OTEL_*` variables in the
//! runner's own environment can never reach a runtime. Variables a child needs are passed
//! explicitly by name.

use std::{ffi::OsStr, process::Stdio};

use tokio::process::Command;

/// A command with an empty environment, no standard input, and `kill_on_drop` so a child never
/// outlives the handle that owns it.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    command.env_clear().stdin(Stdio::null()).kill_on_drop(true);
    command
}
