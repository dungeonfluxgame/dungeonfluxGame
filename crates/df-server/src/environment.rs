//! The only place df-server reads the process environment. It exists only with the `local-dev`
//! feature, so a production build cannot name an environment variable; the scan test in
//! `tests/environment_reads.rs` fails any read outside this module and `command_line.rs`.

use std::path::PathBuf;

/// Names the configuration file of a local development run (`DF_SERVER_CONFIG`).
pub fn config_path() -> Option<PathBuf> {
    std::env::var_os("DF_SERVER_CONFIG").map(PathBuf::from)
}
