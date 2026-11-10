//! The town the party is in: one record of the world state, so the places designed in it and the
//! sights on the way between them are painted as the same town. The first designed place of a move
//! that seeds a town gives its look (materials, street layout, light; no names, since the image
//! model paints any word it is given); later places reuse it; a move that leaves ends it.
//!
//! `GET /dev/world/town` reads it (`world town`). Audit lines:
//! `WORLD town_seeded id=.. created_at=..` and `WORLD town_left id=..`.

use serde::{Deserialize, Serialize};

use super::request::WorldError;
use super::store::{read_town, world_directory, write_town};
use crate::blocking::run_blocking;
use crate::state::AppState;

/// The town the party is in: its id, its look and the move that seeded it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Town {
    pub id: String,
    pub look: String,
    /// The id of the move whose design gave the look (the plate's name).
    pub created_at: String,
}

/// What `GET /dev/world/town` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TownView {
    pub town: Option<Town>,
}

/// The town the party is in now: none when there is none, or the world state cannot be read (one
/// `WORLD town_unavailable error=<key>` line).
pub fn town_of(app: &AppState) -> Option<Town> {
    match read_town(&world_directory(app)) {
        Ok(town) => town,
        Err(error) => {
            app.record_line(format!("WORLD town_unavailable error={}", error.key()));
            None
        }
    }
}

/// Keeps `town` as the one the party is in.
pub async fn seed_town(app: &AppState, town: Town) -> Result<(), WorldError> {
    let directory = world_directory(app);
    let line = format!(
        "WORLD town_seeded id={} created_at={}",
        town.id, town.created_at
    );
    run_blocking(
        move || write_town(&directory, Some(town)),
        WorldError::Stopped,
    )
    .await?;
    app.record_line(line);
    Ok(())
}

/// Ends the town the party is in; answers it when there was one.
pub async fn end_town(app: &AppState) -> Result<Option<Town>, WorldError> {
    let directory = world_directory(app);
    let left = run_blocking(move || write_town(&directory, None), WorldError::Stopped).await?;
    if let Some(town) = &left {
        app.record_line(format!("WORLD town_left id={}", town.id));
    }
    Ok(left)
}
