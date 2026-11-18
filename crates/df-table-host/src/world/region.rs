//! The region the party is in: one record of the world state, so a crossing to another land (an
//! old gate, a teleport) changes where every later place is designed. Entering a region makes the
//! party's place nowhere and clears the town, the places visited and the places kept to go back
//! to: the map starts again in the new land, and the next move designs its arrival there. The
//! design call is told the region (`movement::routes`), so every place after the crossing is of
//! it. The quests stay.
//!
//! A scene memory the runner keeps in a region carries the region's id as a tag, so the phone's
//! map can show the places of the region the party is in (`region_of`).
//!
//! The crossing is marked on the shared screen by a chapter title card (`SceneChange::TitleCard`:
//! "Entering", the region's title, where the party arrives), up from the crossing until the runner
//! clears it once the arrival scene is up and has been read (`POST /dev/world/region/card/clear`).
//!
//! `POST /dev/world/region` enters one, `GET` reads it (`world region`). Audit lines: `REGION
//! changed from=<id|none> to=<id>`, `REGION card_shown to=<id> at_ms=..`, `REGION card_cleared
//! at_ms=..` (or `REGION card_refused status=..`) and `WORLD refused verb=region error=<key>`.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use df_content::world_bible::is_valid_id;
use df_playground::kit::title_card::{CardScale, TitleCard};
use df_playground::scene::SceneChange;
use serde::{Deserialize, Serialize};

use super::request::WorldError;
use super::routes::path;
use super::store::{enter_region, read_region, world_directory};
use crate::blocking::run_blocking;
use crate::dev_http::{parse_body, reply};
use crate::state::AppState;

/// A region of the world: its id, the words of the card that marks the crossing into it, where
/// the party arrives and how the land and its people look (materials, light, dress; no names).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub id: String,
    pub title: String,
    pub arrival: String,
    pub look: String,
}

/// The kicker over the title of the card that marks a crossing.
const CROSSING_KICKER: &str = "Entering";

impl Region {
    /// The chapter card that marks the crossing into this region.
    pub fn card(&self) -> TitleCard {
        TitleCard {
            kicker: Some(CROSSING_KICKER.to_owned()),
            title: Some(self.title.trim().to_owned()),
            text: Some(self.arrival.trim().to_owned()),
            scale: CardScale::Chapter,
            ..TitleCard::default()
        }
    }

    /// The words the design call is told, so a place designed now is of this region.
    pub fn design_words(&self) -> String {
        format!(
            "Every place now is in this land, the same in every picture of it: {}. The party \
             came into it at {}.",
            self.look.trim().trim_end_matches('.'),
            self.arrival.trim().trim_end_matches('.')
        )
    }
}

/// Puts `change` on the shared screen; one `REGION <done> ..` line, or `REGION card_refused` when
/// the screen refuses it (the crossing itself stands).
async fn show_card(app: &AppState, change: SceneChange, done: String) {
    match crate::playground::apply_scene_changes(app, vec![change]).await {
        Ok(_) => app.record_line(format!(
            "REGION {done} at_ms={}",
            app.playground().clock_ms()
        )),
        Err(error) => app.record_line(format!(
            "REGION card_refused status={}",
            error.status.as_u16()
        )),
    }
}

/// What `GET /dev/world/region` answers, and `POST` with the region left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionView {
    pub region: Option<Region>,
    /// The region left by the crossing (POST only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<Region>,
}

fn check_region(region: &Region) -> Result<(), WorldError> {
    let invalid = |reason: String| WorldError::InvalidRegion { reason };
    if !is_valid_id(&region.id) {
        return Err(invalid(format!("'{}' is not an id", region.id)));
    }
    for (field, text) in [
        ("title", &region.title),
        ("arrival", &region.arrival),
        ("look", &region.look),
    ] {
        if text.trim().is_empty() {
            return Err(invalid(format!("the region {} has no {field}", region.id)));
        }
    }
    Ok(())
}

/// The region the party is in now: none before any crossing, or when the world state cannot be
/// read (one `WORLD region_unavailable error=<key>` line).
pub fn region_of(app: &AppState) -> Option<Region> {
    match read_region(&world_directory(app)) {
        Ok(region) => region,
        Err(error) => {
            app.record_line(format!("WORLD region_unavailable error={}", error.key()));
            None
        }
    }
}

async fn get_region(State(app): State<Arc<AppState>>) -> Response {
    let directory = world_directory(&app);
    let result = run_blocking(
        move || read_region(&directory).map(|region| RegionView { region, left: None }),
        WorldError::Stopped,
    )
    .await;
    if let Err(error) = &result {
        app.record_line(format!("WORLD refused verb=region error={}", error.key()));
    }
    reply::<RegionView, WorldError>(result)
}

// TODO(DEV-12): a crossing clears the place, town, visited and kept places and keeps the quests;
// the card shown and cleared, and the REGION lines.
async fn post_region(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let region: Region = parse_body(&bytes)?;
        check_region(&region)?;
        let directory = world_directory(&app);
        let entered = region.clone();
        let left = run_blocking(
            move || enter_region(&directory, entered),
            WorldError::Stopped,
        )
        .await?;
        app.record_line(format!(
            "REGION changed from={} to={}",
            left.as_ref().map_or("none", |left| left.id.as_str()),
            region.id
        ));
        // Terms put in the land left behind are not open in the new one.
        app.deals().leave(&app);
        let card = SceneChange::TitleCard {
            card: region.card(),
        };
        show_card(&app, card, format!("card_shown to={}", region.id)).await;
        Ok::<_, WorldError>(RegionView {
            region: Some(region),
            left,
        })
    }
    .await;
    if let Err(error) = &result {
        app.record_line(format!("WORLD refused verb=region error={}", error.key()));
    }
    reply::<RegionView, WorldError>(result)
}

async fn post_card_clear(State(app): State<Arc<AppState>>) -> Response {
    show_card(
        &app,
        SceneChange::ClearTitleCard {},
        "card_cleared".to_owned(),
    )
    .await;
    reply::<RegionView, WorldError>(Ok(RegionView {
        region: region_of(&app),
        left: None,
    }))
}

/// The region route.
pub(super) fn router_with(app: Arc<AppState>) -> Router {
    Router::new()
        .route(path::WORLD_REGION, post(post_region).get(get_region))
        .route(path::WORLD_REGION_CARD_CLEAR, post(post_card_clear))
        .with_state(app)
}
