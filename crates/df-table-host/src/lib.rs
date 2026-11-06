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
//! `server` (accept them; `playground` serves the playground's files) -> `request` (words to a
//! typed effect) -> `df_table::canvas` (apply and record) -> `sink` and `event_log` (keep the
//! line) -> `view` and `render` (draw it). `wire` holds the types both sides share.
//!
//! The dev route convention (`planning/coding-style.md`, "Dev routes and CLI") is being adopted,
//! not yet true everywhere. The routes that change what the playground shows answer the shared
//! envelope, `dev_http::DevReply`, through `dev_http::reply`. The engine, director, scene, cast,
//! locate and imagegen routes read bodies with `dev_http::parse_body` and their CLIs call through
//! `dev_client`, but each still keeps its own reply type; the canvas and fixture-file routes still
//! answer `wire::Reply`.
//!
//! Differences from section O of the plan are listed in the `df-table` crate docs. The host has no
//! player surfaces yet: this is the workbench the machines and pipeline plug into.

pub mod adapters;
mod atomic_file;
pub mod broker;
pub mod cast;
pub mod client;
pub mod command;
pub mod dev;
mod dev_client;
mod dev_http;
pub mod director;
pub mod engine;
pub mod environment;
pub mod event_log;
pub mod guard;
pub mod imagegen;
pub mod kind_files;
pub mod limits;
pub mod locate;
pub mod machine_wire;
mod media_ingest;
pub mod media_library;
mod media_range;
mod media_store;
mod media_upload;
pub mod observe;
pub mod options;
pub mod owner_latency;
pub mod playground;
pub mod render;
pub mod repl;
pub mod report;
pub mod request;
pub mod scene_builder;
pub mod serve;
pub mod server;
pub mod services;
pub mod sink;
pub mod state;
pub mod text;
mod video_container;
pub mod view;
pub mod wire;

#[cfg(test)]
mod atomic_file_tests;
#[cfg(test)]
mod boot_tests;
#[cfg(test)]
mod camera_tests;
#[cfg(test)]
mod client_tests;
#[cfg(test)]
mod command_tests;
#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod dev_http_tests;
#[cfg(test)]
mod dev_playground_tests;
#[cfg(test)]
mod dev_tests;
#[cfg(test)]
mod echo_tests;
#[cfg(test)]
mod environment_tests;
#[cfg(test)]
mod event_log_tests;
#[cfg(test)]
mod fixture_contract_tests;
#[cfg(test)]
mod frame_tests;
#[cfg(test)]
mod guard_tests;
#[cfg(test)]
mod kind_files_tests;
#[cfg(test)]
mod machine_wire_tests;
#[cfg(test)]
mod media_range_tests;
#[cfg(test)]
mod media_store_tests;
#[cfg(test)]
mod media_tests;
#[cfg(test)]
mod media_upload_tests;
#[cfg(test)]
mod motion_command_tests;
#[cfg(test)]
mod options_tests;
#[cfg(test)]
mod owner_latency_tests;
#[cfg(test)]
mod playground_atomic_tests;
#[cfg(test)]
mod playground_control_tests;
#[cfg(test)]
mod playground_tests;
#[cfg(test)]
mod projection_samples;
#[cfg(test)]
mod render_tests;
#[cfg(test)]
mod repl_tests;
#[cfg(test)]
mod report_tests;
#[cfg(test)]
mod request_tests;
#[cfg(test)]
mod rotation_command_tests;
#[cfg(test)]
mod round_two_tests;
#[cfg(test)]
mod scene_command_tests;
#[cfg(test)]
mod scene_home_configure_host_tests;
#[cfg(test)]
mod scene_home_tests;
#[cfg(test)]
mod scene_lobby_tests;
#[cfg(test)]
mod scene_transition_tests;
#[cfg(test)]
mod server_tests;
#[cfg(test)]
mod sink_tests;
#[cfg(test)]
mod starter_kinds_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod ui_scene_tests;
#[cfg(test)]
mod video_container_tests;
#[cfg(test)]
mod video_fixtures;
#[cfg(test)]
mod video_library_tests;
#[cfg(test)]
mod wire_tests;
