//! The check judge and the dungeon master's checks: whether a roll is called, the call said aloud,
//! the dice rolled and shown on the TV, and the result read in plain words.
//!
//! - `judge`: a model call against a closed schema decides whether a player's action needs a roll,
//!   or whether the narrator wants one called, and which; code validates every field against the
//!   roll vocabulary, the difficulty ladder, the character sheet and what was already rolled.
//! - `rolling`: an accepted call becomes a `df-rules` request from the character sheet.
//! - `numbers`: the numbers and the verdict of a call or a result, composed in code in words.
//! - `lines`: the dungeon master's two moments, `check_called` and `check_result`: the storyteller
//!   words the flourish around the composed slot, and code and judges refuse a line whose numbers
//!   or mood do not fit the roll.
//! - `view`: the conversion from a resolved roll to the display's `RollView`.
//! - `flow`: one check start to finish with its times; `routes` and `cli` expose it.
//! - `seat_roll`: a seat's own roll of a check of its hero sheet (`PLAY_SEAT_ROLL`), on both
//!   listeners.
//!
//! Routes: `POST /dev/persona/check/judge` and `POST /dev/persona/check/run`; verbs:
//! `persona check judge` and `persona check run`. The judge never decides how a roll comes out,
//! the dice come from the host's recording dice (real entropy in play) and every number the table
//! hears is the resolved result's. Rules clauses: SRD 5.2.1 pages 6 to 9 (D20 Tests and the
//! difficulty ladder), 16 to 18 (damage, healing, death saves), 22 and 186 (passive scores), 26
//! to 27 (tables), 183 and 190 (the two contests).

pub(super) mod cli;
mod flow;
mod judge;
mod limits;
mod lines;
mod numbers;
mod pending;
mod request;
mod rolling;
mod routes;
mod seat_roll;
mod sheet;
mod view;
mod vitals;

pub use self::cli::CheckCommand;
pub(crate) use self::pending::{PendingRollError, answer_pending_roll, pending_roll};
pub use self::request::CheckRouteError;
pub(crate) use self::routes::CheckRoutes;
pub(super) use self::routes::router_over;
pub(crate) use self::seat_roll::seat_router;

#[cfg(test)]
mod cli_tests;
#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod flow_tests;
#[cfg(test)]
mod judge_tests;
#[cfg(test)]
mod lines_tests;
#[cfg(test)]
mod numbers_tests;
#[cfg(test)]
mod pending_tests;
#[cfg(test)]
mod seat_roll_tests;
#[cfg(test)]
mod sheet_tests;
#[cfg(test)]
mod view_tests;
