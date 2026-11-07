//! The personas of characters: who each is, what they do and how they talk, stored one file per
//! character for the dev API, a dialogue system and `scene cast`.
//!
//! A persona (`df_content::persona::Persona`) is stored in the playground's `personas` folder as
//! `<id>.persona.json`, whole or not at all, and links to a character of the shown scene by equal
//! text: its id has the grammar of a SAM layout label. A missing file is "no persona", never an
//! error; a file that does not read is a typed error naming the id. The ids in `RESERVED_IDS`
//! (`remove`, `generate`, `all`, ...) are words of the routes and the command line and cannot be
//! persona ids.
//!
//! Routes (each answers a `PersonaReply`, the shared dev envelope with a typed `PersonaError`):
//!
//! - `GET /dev/persona`: a `PersonaSummary` (id, display name, job) of each stored persona.
//! - `GET /dev/persona/{id}?audience=public|owner`: the persona. `public` (the default) is
//!   `Persona::public_view`; `owner` is everything, the character's drives and private backstory
//!   included. The dev API has no password and is for the operator only, so `owner` is not an
//!   access control: a client of the game never reads it.
//! - `POST /dev/persona`: a persona's JSON, checked and stored; it replaces a stored persona of
//!   the same id and answers its summary and whether it replaced or created.
//! - `POST /dev/persona/edit`: typed edits to a stored persona, all together or not at all.
//! - `POST /dev/persona/remove`: `{"id": ...}`, deletes the persona file. Its
//!   `<id>.backfills.json` stays: the record of what was invented is history, and a persona
//!   written later with the same id continues the sequence (owner decision 2026-11-06).
//! - `POST /dev/persona/generate` and `generate-cast`: the text service writes the persona of a
//!   named character of the shown scene, or of all of them at once.
//! - `POST /dev/persona/say`: the text service words a line from a character, cued on the scene.
//!   The character shows the wordless thinking indicator while it works; `speak: true` also
//!   voices the line and starts its clip with its bubble (`speech`).
//! - `POST /dev/persona/ask`, `coverage` and `backfill`, `GET /dev/persona/backfills?id=`: a
//!   question to a character, the judgement of whether the persona answers it, the filling of what
//!   it lacks by adding detail only, and the record of what was added. `coverage: false` skips
//!   the coverage question and the backfill (one model call); `speak: true` voices the line.
//!
//! The dev CLI's `persona` verbs are these routes. `scene cast` also lists the summary of each
//! cast member that has a stored persona; a persona file that does not read is left out of that
//! list (and recorded) and never stops the cast.
//!
//! Every model call is a deliberately fresh operation: a creative request, not an idempotent
//! command, so a retry pays again (owner decision 2026-11-06; see `generate`). Every persona
//! call goes to the persona model (`DF_TEXT_PERSONA_MODEL`, else the default) unless its request
//! names a `model` (see `text`).

pub(crate) mod address;
mod backfill;
pub mod cli;
mod converse;
mod edit_verbs;
mod format;
pub(crate) mod generate;
pub(crate) mod options;
mod plate;
pub mod request;
pub mod routes;
mod storage;

pub use self::cli::{PersonaCommand, parse};
pub use self::converse::ConverseError;
pub(crate) use self::plate::deliver_plate;
pub use self::plate::PlateStep;
pub use self::request::{PersonaError, PersonaSummary};
pub use self::routes::router_with;
pub(crate) use self::storage::summaries_of;
#[cfg(test)]
pub(crate) use self::storage::write_persona;

#[cfg(test)]
mod ask_tests;
#[cfg(test)]
mod backfill_tests;
#[cfg(test)]
mod converse_tests;
#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod generate_tests;
#[cfg(test)]
mod line_facts_tests;
#[cfg(test)]
mod persona_tests;
#[cfg(test)]
mod voice_tests;
