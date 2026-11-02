//! The native host of the table engine, starting with the blank canvas.
//!
//! The host owns everything with an `.await`, a file or a socket: it loads kind files, serves the
//! developer API and the debugging view on the loopback address, and runs the command line that
//! drives them. All state and every rule lives in `df-table`.
//!
//! To run it: `cargo run -p df-table-host --bin df-table -- serve` starts the canvas (the kind
//! directory defaults to `assets/canvas/kinds`, relative to where it is started), and
//! `df-table dev help` lists the commands that drive it. A typical loop is to edit a kind file,
//! run `df-table dev reload`, and spawn elements with `df-table dev spawn` or a `script` file.
//!
//! Reading order for a newcomer, following one command from the terminal to the viewer:
//! `command` (parse the words) -> `dev` (send them) -> `client` (the socket) -> `guard` and
//! `server` (accept them; `playground` serves the playground's files) -> `request` (words to a typed effect) -> `df_table::canvas` (apply and
//! record) -> `sink` and `event_log` (keep the line) -> `view` and `render` (draw it). `wire`
//! holds the types both sides share.
//!
//! Differences from section O of the plan are listed in the `df-table` crate docs. The host has no
//! player surfaces yet: this is the workbench the machines and pipeline plug into.

pub mod client;
pub mod command;
pub mod dev;
pub mod event_log;
pub mod guard;
pub mod kind_files;
pub mod options;
pub mod playground;
pub mod render;
pub mod report;
pub mod request;
pub mod serve;
pub mod server;
pub mod sink;
pub mod state;
pub mod view;
pub mod wire;

#[cfg(test)]
mod client_tests;
#[cfg(test)]
mod command_tests;
#[cfg(test)]
mod dev_playground_tests;
#[cfg(test)]
mod dev_tests;
#[cfg(test)]
mod event_log_tests;
#[cfg(test)]
mod guard_tests;
#[cfg(test)]
mod kind_files_tests;
#[cfg(test)]
mod options_tests;
#[cfg(test)]
mod playground_control_tests;
#[cfg(test)]
mod playground_tests;
#[cfg(test)]
mod render_tests;
#[cfg(test)]
mod report_tests;
#[cfg(test)]
mod request_tests;
#[cfg(test)]
mod server_tests;
#[cfg(test)]
mod sink_tests;
#[cfg(test)]
mod starter_kinds_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod wire_tests;
