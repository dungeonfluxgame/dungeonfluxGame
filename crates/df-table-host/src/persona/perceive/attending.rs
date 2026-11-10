//! What a speaker last turned their attention to, for the frame judge. A perception keeps its
//! answer as a thought (`thoughts`), and a thought whose attention was an item links to that
//! item and to the place the character was at. The judge is told, for each speaker, the item of
//! their newest thought when that thought was had at the place that is active now and the item is
//! still one the judge is offered, so a look line that steps up to the attended thing is read as
//! the close look it is, and not guessed at from the line alone. It expires when the place changes,
//! and a newer thought that attended to nothing, or to a person, ends it. The judge still decides:
//! the id is only a fact it is given.

use serde_json::{Value, json};

use super::super::converse::Speaker;
use super::thoughts::recent_thoughts;
use crate::state::AppState;
use crate::world::PlaceItem;

/// `[{"speaker": <id>, "item": <id>}]` for each of `speakers` whose newest thought attended to one
/// of `items`, at the active place.
pub(crate) async fn attending(app: &AppState, speakers: &[Speaker], items: &[PlaceItem]) -> Value {
    let facts = crate::world::context_for(app, &[]).facts;
    let Some(place) = facts["place"]["id"].as_str() else {
        return Value::Array(Vec::new());
    };
    let mut attending = Vec::new();
    for speaker in speakers {
        let Some(newest) = recent_thoughts(app, &speaker.id, 1).await.pop() else {
            continue;
        };
        if newest.place.as_deref() != Some(place) {
            continue;
        }
        let attended = newest
            .attended
            .filter(|item| items.iter().any(|offered| offered.id == *item));
        if let Some(item) = attended {
            attending.push(json!({"speaker": speaker.id, "item": item}));
        }
    }
    Value::Array(attending)
}
