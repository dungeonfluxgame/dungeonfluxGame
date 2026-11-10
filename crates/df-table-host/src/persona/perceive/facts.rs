//! The facts a perception is asked from, for one character: the picture's scan with each person
//! only as this character knows them, the persona, the state, what they carry, where they are and
//! what is in sight, the thoughts they last had and the last lines of the scene. The stable part
//! comes first (`context`), the part that changes last (`now`), so a prefix cache can hit.
//!
//! Names are learned, never given: a person whose name the character has not learned is a handle
//! (`person-N`) with how they look, never an id or a display name (`persona::handles`). Another
//! character's private side is never in the facts; the character's own is, since it is their own
//! thoughts being asked for.
//!
//! Every list is bounded (see the limits below). A scene bigger than a bound lists the first
//! regions, items and lines that fit and says how many were left out, so a big scene is a larger
//! question and never an unbounded one.

use std::sync::Arc;

use df_content::persona::{BriefScope, Persona, PersonaBrief, PersonaState};
use df_scene_layout::SceneRegion;
use serde_json::{Value, json};

use super::super::generate::{PUBLIC_NOTE, compact, personas_named};
use super::super::handles::Handles;
use super::super::line::state_facts;
use super::super::request::PersonaError;
use super::super::state::load_state;
use super::super::story_next::the_look;
use super::request::{Mode, PerceiveCause, Viewpoint};
use super::screen::Screen;
use super::thoughts::{PlaceNote, REMINDED_OF, recent_thoughts};
use crate::blocking::run_blocking;
use crate::state::AppState;
use crate::world::{InventoryItem, PlaceItem};

/// The most scan regions the facts list; the rest are counted.
pub(super) const MAXIMUM_REGIONS: usize = 48;
/// The most items of the place the facts list.
const MAXIMUM_PLACE_ITEMS: usize = 48;
/// The most kinds of item the character is told they carry.
const MAXIMUM_CARRIED: usize = 48;
/// The most lines of the scene the facts carry (the newest).
const MAXIMUM_RECENT: usize = 30;

/// What the caller asked about the sight.
pub(super) struct Sight<'a> {
    pub(super) mode: Mode,
    pub(super) focus: Option<&'a str>,
    pub(super) recent: &'a [String],
    /// The words the picture was made from, given only when the model is not.
    pub(super) scene: Option<&'a str>,
}

/// What the model may name in its answer, and how to read a handle back.
pub(super) struct Offered {
    pub(super) regions: Vec<String>,
    pub(super) items: Vec<String>,
    /// Every id of the place's items and things and of what the character carries, listed or not:
    /// a new thing takes none of them.
    pub(super) taken: Vec<String>,
    pub(super) handles: Handles,
    /// How each stranger of the scan looks, by persona id.
    pub(super) looks: Vec<(String, String)>,
    pub(super) viewpoint: Viewpoint,
    pub(super) place: Option<PlaceNote>,
}

/// The facts of one perception and what its answer is checked against.
pub(super) struct Briefing {
    pub(super) facts: Value,
    pub(super) offered: Offered,
}

/// What the character's own files and the world say, read together off the async workers.
struct Gathered {
    state: PersonaState,
    place_items: Vec<PlaceItem>,
    held: Vec<InventoryItem>,
    world: Value,
}

fn gather(app: &AppState, who: &str) -> Gathered {
    Gathered {
        state: load_state(app, who),
        place_items: crate::world::place_items(app),
        held: crate::world::inventory_of(app, who)
            .into_iter()
            .filter(|item| item.count > 0)
            .collect(),
        world: crate::world::context_for_viewer(app, &[], who).facts,
    }
}

/// The regions the facts list, in the scan's order, and how many were left out: all of them when
/// they fit, else the character's own and then the surest.
fn listed_regions<'a>(
    regions: &'a [SceneRegion],
    own: &str,
) -> (Vec<(usize, &'a SceneRegion)>, usize) {
    let mut chosen: Vec<(usize, &SceneRegion)> = regions.iter().enumerate().collect();
    let left_out = chosen.len().saturating_sub(MAXIMUM_REGIONS);
    if left_out > 0 {
        chosen.sort_by_key(|(index, region)| {
            (
                region.label() != Some(own),
                std::cmp::Reverse(region.score_permille()),
                *index,
            )
        });
        chosen.truncate(MAXIMUM_REGIONS);
        chosen.sort_by_key(|(index, _)| *index);
    }
    (chosen, left_out)
}

/// The facts of a perception by `own` of `screen`, and the ids its answer may name.
pub(super) async fn briefing(
    app: &Arc<AppState>,
    own: &Persona,
    screen: &Screen,
    sight: &Sight<'_>,
) -> Result<Briefing, PerceiveCause> {
    let who = own.id().as_str().to_owned();
    let gathering = Arc::clone(app);
    let gathered = {
        let who = who.clone();
        run_blocking(
            move || Ok(gather(&gathering, &who)),
            PerceiveCause::from(PersonaError::Stopped),
        )
        .await?
    };
    let Gathered {
        state,
        place_items,
        held,
        world,
    } = gathered;
    let regions: &[SceneRegion] = screen
        .layout
        .as_ref()
        .map_or(&[], |layout| layout.regions());
    // Everyone the scan names, in its order, so a handle means the same person in every prompt of
    // the scene.
    let labels: Vec<String> = regions
        .iter()
        .filter_map(|region| region.label().map(str::to_owned))
        .collect();
    let handles = Handles::new(&state, labels.iter().map(String::as_str));
    let others_named: Vec<String> = labels
        .iter()
        .filter(|label| **label != who)
        .take(MAXIMUM_REGIONS)
        .cloned()
        .collect();
    let others = personas_named(app, others_named).await?;
    let mut looks: Vec<(String, String)> = Vec::new();
    for other in &others {
        let id = other.id().as_str();
        if handles.shown(id) != id {
            looks.push((id.to_owned(), the_look(other.job())));
        }
    }
    let briefs: Vec<PersonaBrief> = others
        .iter()
        .map(|other| handles.listener_brief(other.brief(BriefScope::Listener)))
        .collect();

    let (listed, left_out) = listed_regions(regions, &who);
    let scan: Vec<Value> = listed
        .iter()
        .map(|(number, region)| {
            let called = match region.label() {
                Some(label) if label == who => "you".to_owned(),
                Some(label) => handles.shown(label),
                None => "someone".to_owned(),
            };
            json!({
                "id": region.id().as_str(),
                "number": number,
                "left": region.left_permille(),
                "top": region.top_permille(),
                "right": region.right_permille(),
                "bottom": region.bottom_permille(),
                "who": called,
            })
        })
        .collect();
    let region_ids: Vec<String> = listed
        .iter()
        .map(|(_, region)| region.id().as_str().to_owned())
        .collect();
    let viewpoint = regions
        .iter()
        .find(|region| region.label() == Some(who.as_str()))
        .map_or(Viewpoint::Viewer, |region| Viewpoint::Pictured {
            region: region.id().as_str().to_owned(),
        });

    let items_here: Vec<Value> = place_items
        .iter()
        .take(MAXIMUM_PLACE_ITEMS)
        .map(|item| {
            json!({
                "id": item.id,
                "name": item.name,
                "tags": item.tags,
                "look": item.look,
                "keeper": item.keeper.as_deref().map(|keeper| handles.shown(keeper)),
            })
        })
        .collect();
    let carrying: Vec<Value> = held
        .iter()
        .take(MAXIMUM_CARRIED)
        .map(|item| {
            json!({"id": item.id, "name": item.name, "count": item.count, "use": item.usage})
        })
        .collect();
    let item_ids: Vec<String> = place_items
        .iter()
        .take(MAXIMUM_PLACE_ITEMS)
        .map(|item| item.id.clone())
        .chain(
            held.iter()
                .take(MAXIMUM_CARRIED)
                .map(|item| item.id.clone()),
        )
        .collect();

    let taken: Vec<String> = place_items
        .iter()
        .map(|item| item.id.clone())
        .chain(held.iter().map(|item| item.id.clone()))
        .collect();
    let focus = match sight.focus {
        None => None,
        Some(id) if region_ids.iter().any(|region| region == id) => Some(json!({"region": id})),
        Some(id) if item_ids.iter().any(|item| item == id) => Some(json!({"item": id})),
        Some(id) => {
            return Err(PerceiveCause::UnknownFocus { id: id.to_owned() });
        }
    };

    let place = world.get("place");
    let place_note = place.and_then(|place| {
        Some(PlaceNote {
            id: place.get("id")?.as_str()?.to_owned(),
            name: place.get("name")?.as_str()?.to_owned(),
        })
    });
    let earlier: Vec<String> = recent_thoughts(app, &who, REMINDED_OF)
        .await
        .into_iter()
        .map(|thought| thought.text)
        .collect();
    let skipped = sight.recent.len().saturating_sub(MAXIMUM_RECENT);
    let recent: Vec<String> = sight.recent[skipped..]
        .iter()
        .map(|line| handles.recent_line(line))
        .collect();

    let vision = screen.image.is_some();
    let facts = compact(json!({
        "context": {
            "character": own.brief(BriefScope::Own),
            "others": briefs,
            "public_note": PUBLIC_NOTE,
        },
        "now": {
            "mode": sight.mode.key(),
            "viewpoint": match viewpoint {
                Viewpoint::Pictured { .. } => "in_picture",
                Viewpoint::Viewer => "viewer",
            },
            "place": place,
            "picture": {
                "name": screen.picture,
                "shows": if screen.close_up {
                    "an insert that fills the screen with one thing"
                } else {
                    "the room"
                },
                "attached": vision,
                "scene": sight.scene.filter(|_| !vision),
                "regions_left_out": (left_out > 0).then_some(left_out),
            },
            "scan": scan,
            "items_here": items_here,
            "carrying": carrying,
            "state": state_facts(&handles.state(&state)),
            "earlier_thoughts": earlier,
            "recent": recent,
            "recent_lines_left_out": (skipped > 0).then_some(skipped),
            "focus": focus,
        },
    }));
    Ok(Briefing {
        facts,
        offered: Offered {
            regions: region_ids,
            items: item_ids,
            taken,
            handles,
            looks,
            viewpoint,
            place: place_note,
        },
    })
}
