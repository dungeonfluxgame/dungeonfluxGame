//! The stand-in per-audience projection: an immutable state in, one JSON view per audience out.
//!
//! It is `project(snapshot, audience) -> View` of RES30 in miniature: a pure function over a shared
//! reference that filters the state by what the audience may see (public, its own private data,
//! never hidden data) and serialises the result. A phone view shows its hero, the scene's visible
//! elements, its own private facts, the last public facts and narration, and a list of choices. The
//! TV view shows the public scene, the party and the public facts. Sizes follow the
//! `WorkloadProfile`, and the report states the bytes actually produced.

use std::time::Instant;

use serde::Serialize;

use super::error::HarnessError;
use super::state::{
    AUDIENCE_COUNT, Audience, Element, ElementKind, Fact, TableState, Visibility, WorkloadProfile,
};
use super::statistics::nanoseconds;

const CHOICES: [(&str, &str); 12] = [
    ("look", "Look around"),
    ("listen", "Listen at the door"),
    ("search", "Search the room"),
    ("talk", "Talk to the innkeeper"),
    ("attack", "Attack the nearest raider"),
    ("dodge", "Take the Dodge action"),
    ("dash", "Dash toward the exit"),
    ("hide", "Hide behind the barrels"),
    ("help", "Help an ally"),
    ("ready", "Ready an action"),
    ("use_item", "Use an item"),
    ("end_turn", "End your turn"),
];

#[derive(Serialize)]
struct ElementView<'a> {
    id: u32,
    name: &'a str,
    kind: ElementKind,
    x: i32,
    y: i32,
    hit_points: i32,
    maximum_hit_points: i32,
    conditions: &'a [String],
}

impl<'a> ElementView<'a> {
    fn of(element: &'a Element) -> Self {
        Self {
            id: element.id,
            name: &element.name,
            kind: element.kind,
            x: element.x,
            y: element.y,
            hit_points: element.hit_points,
            maximum_hit_points: element.maximum_hit_points,
            conditions: &element.conditions,
        }
    }
}

#[derive(Serialize)]
struct FactView<'a> {
    id: u32,
    text: &'a str,
}

impl<'a> FactView<'a> {
    fn of(fact: &'a Fact) -> Self {
        Self {
            id: fact.id,
            text: &fact.text,
        }
    }
}

#[derive(Serialize)]
struct NarrationView<'a> {
    id: u32,
    text: &'a str,
}

#[derive(Serialize)]
struct ChoiceView {
    action: &'static str,
    label: &'static str,
}

#[derive(Serialize)]
struct PhoneView<'a> {
    revision: u64,
    scene: u8,
    hero: Option<ElementView<'a>>,
    scene_elements: Vec<ElementView<'a>>,
    private_facts: Vec<FactView<'a>>,
    public_facts: Vec<FactView<'a>>,
    narration: Vec<NarrationView<'a>>,
    choices: Vec<ChoiceView>,
}

#[derive(Serialize)]
struct TvView<'a> {
    revision: u64,
    scene: u8,
    party: Vec<ElementView<'a>>,
    scene_elements: Vec<ElementView<'a>>,
    public_facts: Vec<FactView<'a>>,
    narration: Vec<NarrationView<'a>>,
}

fn visible_scene_elements<'a>(state: &'a TableState, audience: Audience) -> Vec<ElementView<'a>> {
    state
        .elements
        .iter()
        .filter(|element| {
            element.scene == state.scene && element.visibility.is_visible_to(audience)
        })
        .map(ElementView::of)
        .collect()
}

fn last_facts<'a>(
    state: &'a TableState,
    audience: Audience,
    only_private: bool,
    limit: usize,
) -> Vec<FactView<'a>> {
    let mut chosen: Vec<FactView<'a>> = state
        .facts
        .iter()
        .rev()
        .filter(|fact| fact.visibility.is_visible_to(audience))
        .filter(|fact| only_private == (fact.visibility != Visibility::Public))
        .take(limit)
        .map(FactView::of)
        .collect();
    chosen.reverse();
    chosen
}

fn last_narration(state: &TableState, limit: usize) -> Vec<NarrationView<'_>> {
    let skipped = state.narration.len().saturating_sub(limit);
    state
        .narration
        .iter()
        .skip(skipped)
        .map(|line| NarrationView {
            id: line.id,
            text: &line.text,
        })
        .collect()
}

/// The JSON view of `state` for `audience`.
pub fn project(
    state: &TableState,
    profile: &WorkloadProfile,
    audience: Audience,
) -> Result<Vec<u8>, HarnessError> {
    match audience {
        Audience::Seat(seat) => {
            let view = PhoneView {
                revision: state.revision,
                scene: state.scene,
                hero: state.elements.get(usize::from(seat)).map(ElementView::of),
                scene_elements: visible_scene_elements(state, audience),
                private_facts: last_facts(
                    state,
                    audience,
                    true,
                    profile.phone_private_facts_in_view,
                ),
                public_facts: last_facts(state, audience, false, profile.public_facts_in_view),
                narration: last_narration(state, profile.narration_in_view),
                choices: CHOICES
                    .iter()
                    .map(|&(action, label)| ChoiceView { action, label })
                    .collect(),
            };
            Ok(serde_json::to_vec(&view)?)
        }
        Audience::Tv => {
            let view = TvView {
                revision: state.revision,
                scene: state.scene,
                party: state
                    .elements
                    .iter()
                    .filter(|element| element.kind == ElementKind::Hero)
                    .map(ElementView::of)
                    .collect(),
                scene_elements: visible_scene_elements(state, audience),
                public_facts: last_facts(state, audience, false, profile.public_facts_in_view),
                narration: last_narration(state, profile.narration_in_view),
            };
            Ok(serde_json::to_vec(&view)?)
        }
    }
}

/// The views of every audience of one input, with what each cost.
pub struct ProjectedViews {
    pub views: Vec<Vec<u8>>,
    pub per_audience_ns: [u64; AUDIENCE_COUNT],
}

/// Projects every audience in turn, as the owner thread would, timing each.
pub fn project_all(
    state: &TableState,
    profile: &WorkloadProfile,
) -> Result<ProjectedViews, HarnessError> {
    let mut views = Vec::with_capacity(AUDIENCE_COUNT);
    let mut per_audience_ns = [0u64; AUDIENCE_COUNT];
    for (index, spent_ns) in per_audience_ns.iter_mut().enumerate() {
        let started = Instant::now();
        views.push(project(state, profile, Audience::at(index))?);
        *spent_ns = nanoseconds(started.elapsed());
    }
    Ok(ProjectedViews {
        views,
        per_audience_ns,
    })
}
