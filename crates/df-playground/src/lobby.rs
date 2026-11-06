//! The player lobby, the loading screen, the roleplay card and the top bar of the shared screen.
//!
//! `model` holds their data and the view decisions made from it; `change` holds the dev CLI's
//! live edits of them and their range checks. Nothing here draws: `view::lobby`,
//! `view::roleplay` and `view::pills` do, in the browser.

pub mod change;
pub mod model;

#[cfg(test)]
mod change_tests;
#[cfg(test)]
mod model_tests;
