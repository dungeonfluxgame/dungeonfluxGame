//! The thoughts a character has had on seeing the screen, kept in the host's standard memory
//! (`world memory`, kind `thought`) under the character's own id as the owner: one memory for each
//! answer, its description the thought and then what was seen and wanted, its picture the one
//! looked at when that is a library picture, its place the world's active place. A thought is
//! private to its owner; `leak_suspect` answers are tagged `private` so nothing that shows
//! thoughts shows them.
//!
//! Keeping a thought never stops a perception: a memory that cannot be put is one `PERSONA
//! thought_unkept who=.. error=<key>` line and the answer goes out without a memory id.

use df_playground::seat::wire::SeatThought;
use df_table::ports::Clock;
use std::sync::atomic::{AtomicU64, Ordering};

use super::request::{Perceived, Thought};
use crate::observe::WallClock;
use crate::state::AppState;
use crate::world::memory::{LinkKind, Memory, MemoryDraft, MemoryLink};

/// The memory kind every thought has.
pub(crate) const THOUGHT_KIND: &str = "thought";
/// The tag of every kept thought.
const PERCEIVED_TAG: &str = "perceived";
/// The tag of a thought the model reports as stating the character's private side.
const PRIVATE_TAG: &str = "private";
/// Who made the memory.
const CREATED_BY: &str = "perceive";
/// The most thoughts the next perception is reminded of.
pub(super) const REMINDED_OF: usize = 6;

/// Gives each memory made in this process its own id.
static NEXT_THOUGHT: AtomicU64 = AtomicU64::new(0);

/// The place a thought was had at.
pub(super) struct PlaceNote {
    pub id: String,
    pub name: String,
}

/// The id of a new thought: the time and a counter, so two made in one millisecond, or after a
/// restart, never replace one another.
fn thought_id() -> String {
    // Relaxed: the counter only has to hand out distinct numbers.
    let ordinal = NEXT_THOUGHT.fetch_add(1, Ordering::Relaxed);
    format!("thought-{}-{ordinal}", WallClock.now_ms())
}

/// What `perceived` saw (of others, never the character itself) and what it wants.
fn saw_and_wanted(perceived: &Perceived) -> (Vec<String>, Vec<String>) {
    let saw = perceived
        .sees
        .iter()
        .filter(|seen| !seen.subject.is_own())
        .map(|seen| seen.what.trim())
        .filter(|what| !what.is_empty())
        .map(str::to_owned)
        .collect();
    (saw, perceived.wants.clone())
}

/// The words a thought is kept as: the thought, then what was seen and what is wanted.
fn thought_text(perceived: &Perceived) -> String {
    let mut text = perceived.thinking_about.trim().to_owned();
    let (saw, _) = saw_and_wanted(perceived);
    if !saw.is_empty() {
        text.push_str(&format!("\n\nSaw: {}.", saw.join("; ")));
    }
    if !perceived.wants.is_empty() {
        text.push_str(&format!("\nWanted: {}.", perceived.wants.join("; ")));
    }
    text
}

/// `perceived` as the memory draft it is kept as.
fn draft_of(perceived: &Perceived, place: Option<&PlaceNote>) -> MemoryDraft {
    let mut links: Vec<MemoryLink> = Vec::new();
    if let super::request::Attended::Item { id } = &perceived.attention {
        links.push(MemoryLink {
            what: LinkKind::Item,
            id: id.clone(),
        });
    }
    if let Some(place) = place {
        links.push(MemoryLink {
            what: LinkKind::Place,
            id: place.id.clone(),
        });
    }
    let mut tags = vec![PERCEIVED_TAG.to_owned(), perceived.mode.key().to_owned()];
    if perceived.leak_suspect {
        tags.push(PRIVATE_TAG.to_owned());
    }
    // The picture was read from the library to be looked at, so the memory may name it.
    let pictures: Vec<String> = perceived.picture.iter().cloned().collect();
    MemoryDraft {
        id: thought_id(),
        owner: perceived.who.clone(),
        kind: THOUGHT_KIND.to_owned(),
        title: place.map_or_else(
            || "Looking at the screen".to_owned(),
            |place| format!("Looking at {}", place.name),
        ),
        description: thought_text(perceived),
        place: place.map(|place| place.id.clone()),
        pictures,
        links,
        tags,
        created_by: Some(CREATED_BY.to_owned()),
    }
}

/// Keeps `perceived` as a thought of its character. The memory's id on success; on failure one
/// `PERSONA thought_unkept` line and none. A thought that is not the character's private side is
/// also handed, in its parts, to the seat that plays the character (its Story page).
pub(super) async fn keep_thought(
    app: &AppState,
    perceived: &Perceived,
    place: Option<&PlaceNote>,
) -> Option<String> {
    if !perceived.leak_suspect {
        let (saw, wanted) = saw_and_wanted(perceived);
        let thought = SeatThought {
            thought: perceived.thinking_about.trim().to_owned(),
            saw,
            wanted,
        };
        app.seats()
            .talk()
            .think(&perceived.who, perceived.picture.clone(), thought);
    }
    let draft = draft_of(perceived, place);
    match crate::world::memory::keep(app, draft).await {
        Ok(put) => Some(put.memory.id),
        Err(error) => {
            app.record_line(format!(
                "PERSONA thought_unkept who={} error={}",
                perceived.who,
                error.key()
            ));
            None
        }
    }
}

/// `memory` as a thought.
pub(super) fn thought_of(memory: Memory) -> Thought {
    Thought {
        id: memory.id,
        seq: memory.seq,
        place: memory.place,
        picture: memory.pictures.first().cloned(),
        attended: memory
            .links
            .iter()
            .find(|link| link.what == LinkKind::Item)
            .map(|link| link.id.clone()),
        text: memory.description,
    }
}

/// The newest `count` thoughts of `who`, oldest first. A store that does not read is one `PERSONA
/// thought_unreadable` line and none.
pub(crate) async fn recent_thoughts(app: &AppState, who: &str, count: usize) -> Vec<Thought> {
    match crate::world::memory::recent(app, who, THOUGHT_KIND, count).await {
        Ok(memories) => memories.into_iter().map(thought_of).collect(),
        Err(error) => {
            app.record_line(format!(
                "PERSONA thought_unreadable who={who} error={}",
                error.key()
            ));
            Vec::new()
        }
    }
}
