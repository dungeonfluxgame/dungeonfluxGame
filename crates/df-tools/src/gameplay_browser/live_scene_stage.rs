//! The audience-safe scene graph and its DOM composite on the TV.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
};

use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Document, Element, Event};

use super::{node, rpc};
#[path = "scene_placement.rs"]
mod scene_placement;
use scene_placement::{Dimensions, placement};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay_browser) enum Camera {
    Wide,
    TwoShot,
    Speaker,
}

impl Camera {
    fn name(self) -> &'static str {
        match self {
            Self::Wide => "wide",
            Self::TwoShot => "two-shot",
            Self::Speaker => "speaker",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay_browser) struct SceneGraphView {
    pub(in crate::gameplay_browser) revision: u64,
    pub(in crate::gameplay_browser) scene_key: String,
    pub(in crate::gameplay_browser) set: SceneSetView,
    pub(in crate::gameplay_browser) camera: Camera,
    pub(in crate::gameplay_browser) actors: Vec<SceneActorView>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay_browser) struct SceneSetView {
    pub(in crate::gameplay_browser) place_name: String,
    pub(in crate::gameplay_browser) asset: String,
    pub(in crate::gameplay_browser) fallback_asset: String,
    pub(in crate::gameplay_browser) exterior: bool,
    pub(in crate::gameplay_browser) time_of_day: String,
    pub(in crate::gameplay_browser) weather: Option<String>,
    pub(in crate::gameplay_browser) lights: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay_browser) struct SceneActorView {
    pub(in crate::gameplay_browser) id: String,
    pub(in crate::gameplay_browser) name: String,
    pub(in crate::gameplay_browser) art: String,
    pub(in crate::gameplay_browser) fallback_art: String,
    pub(in crate::gameplay_browser) spot: String,
    pub(in crate::gameplay_browser) anchor_x_permille: u32,
    pub(in crate::gameplay_browser) anchor_y_permille: u32,
    pub(in crate::gameplay_browser) depth_permille: u32,
    pub(in crate::gameplay_browser) pose: String,
    pub(in crate::gameplay_browser) expression: String,
    pub(in crate::gameplay_browser) look_at: Option<String>,
    pub(in crate::gameplay_browser) speaking: bool,
}

impl SceneGraphView {
    pub(super) fn from_wire(graph: &rpc::DisplaySceneGraph) -> Option<Self> {
        let set = graph.set.as_ref()?;
        Some(Self {
            revision: graph.revision,
            scene_key: graph.scene_key.clone(),
            set: SceneSetView {
                place_name: set.place_name.clone(),
                asset: set.asset.clone(),
                fallback_asset: set.fallback_asset.clone(),
                exterior: set.exterior,
                time_of_day: set.time_of_day.clone(),
                weather: (!set.weather.is_empty()).then(|| set.weather.clone()),
                lights: set.lights.clone(),
            },
            camera: match rpc::DisplaySceneCamera::try_from(graph.camera).ok()? {
                rpc::DisplaySceneCamera::Wide => Camera::Wide,
                rpc::DisplaySceneCamera::TwoShot => Camera::TwoShot,
                rpc::DisplaySceneCamera::Speaker => Camera::Speaker,
                rpc::DisplaySceneCamera::Unspecified => return None,
            },
            actors: graph
                .actors
                .iter()
                .map(|actor| SceneActorView {
                    id: actor.id.clone(),
                    name: actor.name.clone(),
                    art: crate::presentation::set_picture(&actor.art).unwrap_or_default(),
                    fallback_art: crate::presentation::set_picture(&actor.fallback_art)
                        .unwrap_or_default(),
                    spot: actor.spot.clone(),
                    anchor_x_permille: actor.anchor_x_permille,
                    anchor_y_permille: actor.anchor_y_permille,
                    depth_permille: actor.depth_permille,
                    pose: actor.pose.clone(),
                    expression: actor.expression.clone(),
                    look_at: actor.look_at.clone(),
                    speaking: actor.speaking,
                })
                .collect(),
        })
    }
}

type ImageCallbacks = (Closure<dyn FnMut(Event)>, Closure<dyn FnMut(Event)>);

struct ActorLayer {
    root: Element,
    images: Vec<Element>,
    listeners: Vec<ImageCallbacks>,
}

impl Drop for ActorLayer {
    fn drop(&mut self) {
        for (image, (load, error)) in self.images.iter().zip(&self.listeners) {
            let _ =
                image.remove_event_listener_with_callback("load", load.as_ref().unchecked_ref());
            let _ =
                image.remove_event_listener_with_callback("error", error.as_ref().unchecked_ref());
        }
    }
}

struct LayerChange {
    what: String,
    when_ms: f64,
    why: String,
}

pub(super) struct StageLayer {
    root: Element,
    camera: Element,
    set: Element,
    actors_root: Element,
    atmosphere: Element,
    actors: RefCell<Vec<ActorLayer>>,
    changes: RefCell<VecDeque<LayerChange>>,
    revision: Cell<Option<u64>>,
    last_graph: RefCell<Option<SceneGraphView>>,
    scene_key: RefCell<String>,
}

impl StageLayer {
    pub(super) fn mount(document: &Document, tv_root: &Element) -> Result<Self, JsValue> {
        let root = node(document, tv_root, "div", "tv-scene-graph", "")?;
        root.set_attribute("aria-hidden", "true")?;
        root.set_attribute("data-inactive", "")?;
        let camera = node(document, &root, "div", "tv-scene-camera", "")?;
        let set = node(document, &camera, "div", "tv-scene-set", "")?;
        let actors_root = node(document, &camera, "div", "tv-scene-actors", "")?;
        let atmosphere = node(document, &root, "div", "tv-scene-atmosphere", "")?;
        Ok(Self {
            root,
            camera,
            set,
            actors_root,
            atmosphere,
            actors: RefCell::new(Vec::new()),
            changes: RefCell::new(VecDeque::with_capacity(30)),
            revision: Cell::new(None),
            last_graph: RefCell::new(None),
            scene_key: RefCell::new(String::new()),
        })
    }

    /// Applies a graph after the accepted display view. A mismatched plate is ignored immediately.
    pub(super) fn update(
        &self,
        tv_root: &Element,
        graph: Option<&SceneGraphView>,
        baked_revision: Option<u64>,
    ) -> Result<(), JsValue> {
        let Some(graph) = graph else {
            self.root.set_attribute("data-inactive", "")?;
            tv_root.remove_attribute("data-stage-graph")?;
            tv_root.remove_attribute("data-stage-frame-revision")?;
            self.set.remove_attribute("data-asset")?;
            if self.revision.replace(None).is_some() {
                for layer in self.actors.borrow().iter() {
                    actor_timing(&layer.root, "figure-hidden", "scene-change");
                }
                self.last_graph.borrow_mut().take();
                self.scene_key.borrow_mut().clear();
                self.record_change(tv_root, "graph", "scene graph removed from display")?;
            }
            return Ok(());
        };
        validate(graph)?;
        let previous_revision = self.revision.get();
        let previous_scene = self.scene_key.borrow().clone();
        if self.last_graph.borrow().as_ref() == Some(graph) {
            refresh_timing_visibility(&self.root);
            let revision = baked_revision.map(|revision| revision.to_string());
            if tv_root.get_attribute("data-stage-frame-revision") != revision {
                match revision {
                    Some(revision) => {
                        tv_root.set_attribute("data-stage-frame-revision", &revision)?
                    }
                    None => tv_root.remove_attribute("data-stage-frame-revision")?,
                }
            }
            return Ok(());
        }
        self.root.remove_attribute("data-inactive")?;
        self.root
            .set_attribute("data-revision", &graph.revision.to_string())?;
        let previous_camera = self.root.get_attribute("data-camera");
        self.root
            .set_attribute("data-camera", graph.camera.name())?;
        self.root
            .set_attribute("data-time", &graph.set.time_of_day)?;
        self.root.set_attribute(
            "data-weather",
            graph.set.weather.as_deref().unwrap_or("clear"),
        )?;
        self.root.set_attribute(
            "data-exterior",
            if graph.set.exterior { "true" } else { "false" },
        )?;
        self.root
            .set_attribute("data-light", &graph.set.lights.join(" "))?;
        let previous_set = self.set.get_attribute("data-asset");
        self.set
            .set_attribute("data-place", &graph.set.place_name)?;
        self.set.set_attribute("data-asset", &graph.set.asset)?;
        let camera_target = camera_target(graph);
        self.camera.set_attribute(
            "style",
            &format!(
                "--camera-x:{}%;--camera-y:{}%",
                camera_target.0, camera_target.1
            ),
        )?;
        self.atmosphere.set_attribute(
            "data-weather",
            graph.set.weather.as_deref().unwrap_or("clear"),
        )?;
        tv_root.set_attribute("data-stage-graph", "")?;
        if previous_set.as_deref() != Some(graph.set.asset.as_str()) {
            self.record_change(tv_root, "set", "scene place or set art changed")?;
        }
        if previous_camera.as_deref() != Some(graph.camera.name()) {
            self.record_change(tv_root, "camera", "director framing changed")?;
        }

        self.revision.set(Some(graph.revision));
        *self.scene_key.borrow_mut() = graph.scene_key.clone();
        if previous_revision != Some(graph.revision) {
            self.record_change(
                tv_root,
                "graph",
                if previous_scene != graph.scene_key {
                    "new scene graph"
                } else {
                    "visible scene state changed"
                },
            )?;
        }
        if previous_revision.is_none() || previous_scene != graph.scene_key {
            let tick = if self.root.get_attribute("data-live-camera").as_deref() == Some("a") {
                "b"
            } else {
                "a"
            };
            self.root.set_attribute("data-live-camera", tick)?;
        }
        // A plate belongs to the scene, not to its latest speaker/expression revision.
        // Keep the current plate while a settled visible-state edit loads off screen.
        if let Some(revision) = baked_revision {
            tv_root.set_attribute("data-stage-frame-revision", &revision.to_string())?;
        } else {
            tv_root.remove_attribute("data-stage-frame-revision")?;
        }

        let mut actor_layers = self.actors.borrow_mut();
        let scene_changed = previous_revision.is_some() && previous_scene != graph.scene_key;
        let mut actor_index = 0;
        while actor_index < actor_layers.len() {
            let layer = &actor_layers[actor_index];
            let actor_id = layer
                .root
                .get_attribute("data-actor-id")
                .unwrap_or_default();
            let actor_present = graph.actors.iter().any(|actor| {
                actor.id == actor_id && (!actor.art.is_empty() || !actor.fallback_art.is_empty())
            });
            if !scene_changed && actor_present {
                actor_index += 1;
                continue;
            }
            let old_layer = actor_layers.remove(actor_index);
            let actor_name = old_layer
                .root
                .get_attribute("data-name")
                .unwrap_or_default();
            old_layer.root.remove();
            actor_timing(
                &old_layer.root,
                "figure-hidden",
                if scene_changed {
                    "scene-change"
                } else {
                    "left"
                },
            );
            self.record_change(
                tv_root,
                &format!("figure:{actor_name}:present=false"),
                if scene_changed {
                    "new scene replaces actor"
                } else {
                    "actor left scene"
                },
            )?;
        }
        for actor in &graph.actors {
            if actor.art.is_empty() && actor.fallback_art.is_empty() {
                continue;
            }
            let index = actor_layers.iter().position(|layer| {
                layer.root.get_attribute("data-actor-id").as_deref() == Some(&actor.id)
            });
            let layer = match index {
                Some(index) => &mut actor_layers[index],
                None => {
                    actor_layers.push(mount_actor(
                        &self
                            .root
                            .owner_document()
                            .ok_or("display document absent")?,
                        &self.actors_root,
                        actor,
                        &graph.scene_key,
                    )?);
                    self.record_change(
                        tv_root,
                        &format!("figure:{}:present=true", actor.name),
                        "actor entered scene",
                    )?;
                    actor_layers.last_mut().ok_or("scene actor layer absent")?
                }
            };
            let anchor_x = actor.anchor_x_permille.to_string();
            let anchor_y = actor.anchor_y_permille.to_string();
            let changed_spot = layer.root.get_attribute("data-spot").as_deref()
                != Some(&actor.spot)
                || layer.root.get_attribute("data-anchor-x").as_deref() != Some(anchor_x.as_str())
                || layer.root.get_attribute("data-anchor-y").as_deref() != Some(anchor_y.as_str());
            let previous_expression = layer.root.get_attribute("data-expression");
            let previous_pose = layer.root.get_attribute("data-pose");
            update_actor(layer, actor, changed_spot, graph)?;
            if changed_spot {
                self.record_change(
                    tv_root,
                    &format!("actor:{}@{}", actor.name, actor.spot),
                    "spot or anchor changed",
                )?;
            }
            if previous_expression.as_deref() != Some(actor.expression.as_str())
                || previous_pose.as_deref() != Some(actor.pose.as_str())
            {
                self.record_change(
                    tv_root,
                    &format!("actor:{}:{}", actor.name, actor.expression),
                    "expression or pose changed",
                )?;
            }
        }
        *self.last_graph.borrow_mut() = Some(graph.clone());
        refresh_timing_visibility(&self.root);
        Ok(())
    }

    fn record_change(&self, tv_root: &Element, what: &str, why: &str) -> Result<(), JsValue> {
        let mut changes = self.changes.borrow_mut();
        changes.push_back(LayerChange {
            what: what.to_owned(),
            when_ms: js_sys::Date::now(),
            why: why.to_owned(),
        });
        while changes.len() > 30 {
            changes.pop_front();
        }
        let serialized = format!(
            "[{}]",
            changes
                .iter()
                .map(|change| format!(
                    "{{\"what\":\"{}\",\"when_ms\":{:.0},\"why\":\"{}\"}}",
                    json_string(&change.what),
                    change.when_ms,
                    json_string(&change.why)
                ))
                .collect::<Vec<_>>()
                .join(",")
        );
        tv_root.set_attribute("data-stage-log", &serialized)
    }
}

fn validate(graph: &SceneGraphView) -> Result<(), JsValue> {
    if graph.scene_key.is_empty()
        || graph.set.place_name.is_empty()
        || graph.set.asset.is_empty()
        || graph.set.fallback_asset.is_empty()
    {
        return Err(JsValue::from_str("scene graph has empty set data"));
    }
    for (index, actor) in graph.actors.iter().enumerate() {
        if actor.id.is_empty()
            || actor.name.is_empty()
            || actor.anchor_x_permille > 1000
            || actor.anchor_y_permille > 1000
            || actor.depth_permille > 1000
        {
            return Err(JsValue::from_str("scene graph has invalid actor data"));
        }
        if graph.actors[..index]
            .iter()
            .any(|other| other.id == actor.id)
        {
            return Err(JsValue::from_str("scene graph has duplicate actor id"));
        }
    }
    Ok(())
}

fn camera_target(graph: &SceneGraphView) -> (u32, u32) {
    let focus = match graph.camera {
        Camera::Wide => return (50, 50),
        Camera::Speaker => graph
            .actors
            .iter()
            .filter(|actor| actor.speaking)
            .collect::<Vec<_>>(),
        Camera::TwoShot => {
            let speaker = graph.actors.iter().find(|actor| actor.speaking);
            let listener_id = speaker.and_then(|actor| actor.look_at.as_deref());
            graph
                .actors
                .iter()
                .filter(|actor| actor.speaking || listener_id == Some(actor.id.as_str()))
                .collect::<Vec<_>>()
        }
    };
    if focus.is_empty() {
        return (50, 50);
    }
    let x = focus
        .iter()
        .map(|actor| actor.anchor_x_permille)
        .sum::<u32>()
        / focus.len() as u32;
    let y = focus
        .iter()
        .map(|actor| actor.anchor_y_permille)
        .sum::<u32>()
        / focus.len() as u32;
    (x / 10, y / 10)
}

fn mount_actor(
    document: &Document,
    parent: &Element,
    actor: &SceneActorView,
    scene_key: &str,
) -> Result<ActorLayer, JsValue> {
    let root = node(document, parent, "div", "tv-scene-actor", "")?;
    root.set_attribute("data-actor-id", &actor.id)?;
    root.set_attribute("data-scene-key", scene_key)?;
    root.set_attribute("data-absent", "false")?;
    root.set_attribute("data-timing-visible", "false")?;
    let shadow = node(document, &root, "span", "tv-scene-shadow", "")?;
    shadow.set_attribute("aria-hidden", "true")?;
    let body = node(document, &root, "span", "tv-scene-body", "")?;
    let mut images = Vec::with_capacity(2);
    let mut listeners = Vec::with_capacity(2);
    for index in 0..2 {
        let image = node(document, &body, "img", "tv-scene-actor-art", "")?;
        image.set_attribute("alt", "")?;
        image.set_attribute("decoding", "async")?;
        image.set_attribute("data-i", if index == 0 { "0" } else { "1" })?;
        let load = {
            let (root, image) = (root.clone(), image.clone());
            Closure::wrap(Box::new(move |_: Event| {
                if root.get_attribute("data-art-want").as_deref()
                    == image.get_attribute("data-graph-art").as_deref()
                {
                    let activated = root.set_attribute(
                        "data-active",
                        &image.get_attribute("data-i").unwrap_or_default(),
                    );
                    let _ = root.remove_attribute("data-art-unready");
                    if activated.is_ok() && actor_can_show(&root) {
                        actor_timing(&root, "figure-shown", "ready");
                    }
                }
            }) as Box<dyn FnMut(Event)>)
        };
        let error = {
            let (root, image) = (root.clone(), image.clone());
            Closure::wrap(Box::new(move |_: Event| {
                let wanted = root.get_attribute("data-art-want");
                if wanted.as_deref() != image.get_attribute("data-graph-art").as_deref() {
                    return;
                }
                actor_timing(
                    &root,
                    if root.has_attribute("data-active") {
                        "figure-kept"
                    } else {
                        "figure-hidden"
                    },
                    "failed",
                );
                let fallback = image.get_attribute("data-fallback").unwrap_or_default();
                if image.get_attribute("data-source").as_deref() != Some(fallback.as_str()) {
                    let _ = image.set_attribute("data-source", &fallback);
                    let _ = image.set_attribute("src", &fallback);
                } else if !root.has_attribute("data-active") {
                    let _ = root.set_attribute("data-art-unready", "");
                }
            }) as Box<dyn FnMut(Event)>)
        };
        image.add_event_listener_with_callback("load", load.as_ref().unchecked_ref())?;
        image.add_event_listener_with_callback("error", error.as_ref().unchecked_ref())?;
        images.push(image);
        listeners.push((load, error));
    }
    actor_timing(&root, "figure-mounted", "appeared");
    Ok(ActorLayer {
        root,
        images,
        listeners,
    })
}

// Mirror the graph's visibility gates in live_stage.css and the figure-over-plate override
// in live_plate.css. These are DOM visibility transition starts, not pixel observations.
fn actor_can_show(root: &Element) -> bool {
    root.is_connected()
        && root.has_attribute("data-active")
        && root.get_attribute("data-absent").as_deref() != Some("true")
        && root.closest("[hidden],.tv-scene-graph[data-inactive]").is_ok_and(|hidden| hidden.is_none())
        && root.closest(concat!(
                ".tv-stage[data-tv-mode=lobby],.tv-stage[data-tv-mode=combat],",
                ".tv-stage[data-arrange=combat],.tv-stage[data-battlefield-live=true],",
                ".tv-stage[data-battle-state]:not([data-battle-state=absent]),.tv-stage[data-shot=plate]"
            )).is_ok_and(|hidden| hidden.is_none())
}

/// Also call after TV mode/shot/battle visibility changes that do not deliver a new graph.
pub(super) fn refresh_timing_visibility(tv_root: &Element) {
    let Ok(Some(actors)) = tv_root.query_selector(".tv-scene-actors") else {
        return;
    };
    let mut next = actors.first_element_child();
    while let Some(root) = next {
        next = root.next_element_sibling();
        let shown = actor_can_show(&root);
        actor_timing(
            &root,
            if shown {
                "figure-shown"
            } else {
                "figure-hidden"
            },
            if shown { "appeared" } else { "scene-change" },
        );
    }
}

fn actor_timing(root: &Element, kind: &'static str, reason: &'static str) {
    use df_observe::presentation::{TimingEvent, capture_display, local_identity};
    let Some(clock) = web_sys::window().and_then(|window| window.performance()) else {
        web_sys::console::warn_1(&"Figure timing clock unavailable".into());
        return;
    };
    let (Some(scene), Some(actor)) = (
        root.get_attribute("data-scene-key"),
        root.get_attribute("data-actor-id"),
    ) else {
        web_sys::console::warn_1(&"Figure timing identity unavailable".into());
        return;
    };
    if kind == "figure-shown" && !actor_can_show(root) {
        return;
    }
    let visible = match kind {
        "figure-shown" => Some("true"),
        "figure-hidden" => Some("false"),
        _ => None,
    };
    if let Some(visible) = visible {
        if root.get_attribute("data-timing-visible").as_deref() == Some(visible)
            && reason != "failed"
        {
            return;
        }
        if root.set_attribute("data-timing-visible", visible).is_err() {
            web_sys::console::warn_1(&"Figure timing marker unavailable".into());
            return;
        }
    }
    let mut event = TimingEvent::lifecycle(
        kind,
        reason,
        local_identity(&actor),
        local_identity(&scene),
        clock.now() as u64,
    );
    // This records the selection edge; the actor-container fade completes 600 ms later.
    if kind == "figure-hidden"
        && root
            .closest(".tv-stage[data-tv-mode=scene][data-shot=plate]")
            .is_ok_and(|plate| plate.is_some())
    {
        event.crossfade_ms = 600;
    }
    if let Err(error) = capture_display(event) {
        web_sys::console::warn_1(&format!("Figure timing capture refused: {error:?}").into());
    }
}

fn update_actor(
    layer: &mut ActorLayer,
    actor: &SceneActorView,
    changed_spot: bool,
    graph: &SceneGraphView,
) -> Result<(), JsValue> {
    let root = &layer.root;
    let previous_art = root.get_attribute("data-art-want");
    root.set_attribute("data-scene-key", &graph.scene_key)?;
    let previous_expression = root.get_attribute("data-expression");
    let previous_pose = root.get_attribute("data-pose");
    root.set_attribute("data-name", &actor.name)?;
    root.set_attribute("data-spot", &actor.spot)?;
    root.set_attribute("data-anchor-x", &actor.anchor_x_permille.to_string())?;
    root.set_attribute("data-anchor-y", &actor.anchor_y_permille.to_string())?;
    root.set_attribute("data-expression", &actor.expression)?;
    root.set_attribute("data-pose", &actor.pose)?;
    root.set_attribute("data-light", &graph.set.lights.join(" "))?;
    root.set_attribute("data-depth", &actor.depth_permille.to_string())?;
    root.set_attribute(
        "data-speaking",
        if actor.speaking { "true" } else { "false" },
    )?;
    let facing = actor
        .look_at
        .as_deref()
        .and_then(|target| graph.actors.iter().find(|candidate| candidate.id == target))
        .map(|target| {
            if target.anchor_x_permille < actor.anchor_x_permille {
                "left"
            } else {
                "right"
            }
        })
        .unwrap_or("camera");
    root.set_attribute("data-facing", facing)?;
    if let Some(target) = &actor.look_at {
        root.set_attribute("data-look-at", target)?;
    } else {
        root.remove_attribute("data-look-at")?;
    }
    let frame = Dimensions {
        width: 1920.0,
        height: 1080.0,
    };
    let picture = Dimensions {
        width: 16.0,
        height: 9.0,
    };
    let position = placement(
        actor.anchor_x_permille,
        actor.anchor_y_permille,
        actor.depth_permille,
        frame,
        picture,
    )
    .ok_or_else(|| JsValue::from_str("scene actor has invalid placement"))?;
    root.set_attribute(
        "style",
        &format!(
            "--actor-left:{:.3}%;--actor-top:{:.3}%;--actor-scale:{:.3};--depth-brightness:{:.3};--depth-blur:{:.3}px;z-index:{}",
            position.left_percent,
            position.top_percent,
            position.scale,
            0.78 + f64::from(actor.depth_permille) / 1000.0 * 0.22,
            (1.0 - f64::from(actor.depth_permille) / 1000.0) * 1.2,
            position.z_index
        ),
    )?;
    if changed_spot {
        let tick = if root.get_attribute("data-walk").as_deref() == Some("a") {
            "b"
        } else {
            "a"
        };
        root.set_attribute("data-walk", tick)?;
    }
    if previous_expression.as_deref() != Some(actor.expression.as_str())
        || previous_pose.as_deref() != Some(actor.pose.as_str())
    {
        let tick = if root.get_attribute("data-cut").as_deref() == Some("a") {
            "b"
        } else {
            "a"
        };
        root.set_attribute("data-cut", tick)?;
    }
    if previous_art.as_deref() != Some(actor.art.as_str()) {
        root.set_attribute("data-art-want", &actor.art)?;
        let inactive = if root.get_attribute("data-active").as_deref() == Some("0") {
            1
        } else {
            0
        };
        let image = layer
            .images
            .get(inactive)
            .ok_or("scene actor art layer absent")?;
        image.set_attribute("data-fallback", &actor.fallback_art)?;
        image.set_attribute("data-graph-art", &actor.art)?;
        image.set_attribute("data-source", &actor.art)?;
        image.set_attribute("src", &actor.art)?;
        if !root.has_attribute("data-active") {
            root.set_attribute("data-art-unready", "")?;
        }
    }
    Ok(())
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character <= '\u{1f}' => {
                use std::fmt::Write;
                let _ = write!(escaped, "\\u{:04x}", u32::from(character));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> SceneGraphView {
        let image =
            "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
        SceneGraphView {
            revision: 1,
            scene_key: "taproom".into(),
            set: SceneSetView {
                place_name: "Taproom".into(),
                asset: image.into(),
                fallback_asset: image.into(),
                exterior: false,
                time_of_day: "day".into(),
                weather: None,
                lights: vec![],
            },
            camera: Camera::Wide,
            actors: vec![SceneActorView {
                id: "ilsa".into(),
                name: "Ilsa".into(),
                art: image.into(),
                fallback_art: image.into(),
                spot: "bar".into(),
                anchor_x_permille: 500,
                anchor_y_permille: 700,
                depth_permille: 500,
                pose: "standing".into(),
                expression: "neutral".into(),
                look_at: None,
                speaking: false,
            }],
        }
    }

    #[test]
    fn figure_timing_distinguishes_mounted_from_loaded_and_records_scene_removal() {
        use df_observe::presentation::drain_display_capture;
        while !drain_display_capture().0.is_empty() {}
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let layer = StageLayer::mount(&document, &root).unwrap();
        let graph = graph();
        layer.update(&root, Some(&graph), Some(1)).unwrap();
        let (events, lost) = drain_display_capture();
        assert_eq!(lost, 0);
        assert!(events.iter().any(|event| event.kind == "figure-mounted"));
        assert!(!events.iter().any(|event| event.kind == "figure-shown"));
        layer.update(&root, Some(&graph), Some(1)).unwrap();
        assert!(drain_display_capture().0.is_empty());
        layer.actors.borrow()[0]
            .root
            .set_attribute("data-active", "0")
            .unwrap();
        layer.update(&root, Some(&graph), Some(1)).unwrap();
        assert!(
            drain_display_capture()
                .0
                .iter()
                .any(|event| event.kind == "figure-shown")
        );
        layer.update(&root, None, None).unwrap();
        assert!(
            drain_display_capture()
                .0
                .iter()
                .any(|event| event.kind == "figure-hidden" && event.reason == "scene-change")
        );
        // A previously loaded sprite can reappear without another image load event.
        layer.actors.borrow()[0]
            .root
            .set_attribute("data-active", "0")
            .unwrap();
        layer.update(&root, Some(&graph), Some(1)).unwrap();
        assert!(
            drain_display_capture()
                .0
                .iter()
                .any(|event| event.kind == "figure-shown")
        );
        root.remove();
    }

    #[test]
    fn loaded_figures_follow_tv_visibility_even_when_the_graph_is_unchanged() {
        use df_observe::presentation::drain_display_capture;
        while !drain_display_capture().0.is_empty() {}
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        root.set_attribute("class", "tv-stage").unwrap();
        root.set_attribute("data-tv-mode", "combat").unwrap();
        root.set_attribute("data-battle-state", "active").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let layer = StageLayer::mount(&document, &root).unwrap();
        let graph = graph();
        layer.update(&root, Some(&graph), Some(1)).unwrap();
        drain_display_capture();
        let actor = layer.actors.borrow()[0].root.clone();
        actor.set_attribute("data-active", "0").unwrap();
        actor_timing(&actor, "figure-shown", "ready");
        assert!(
            !drain_display_capture()
                .0
                .iter()
                .any(|event| event.kind == "figure-shown")
        );

        root.set_attribute("data-tv-mode", "scene").unwrap();
        root.set_attribute("data-battle-state", "absent").unwrap();
        root.set_attribute("data-shot", "plate").unwrap();
        layer.update(&root, Some(&graph), Some(1)).unwrap();
        assert!(drain_display_capture().0.is_empty());
        root.set_attribute("data-shot", "stage").unwrap();
        layer.update(&root, Some(&graph), Some(1)).unwrap();
        assert!(
            drain_display_capture()
                .0
                .iter()
                .any(|event| event.kind == "figure-shown")
        );
        layer.update(&root, Some(&graph), Some(1)).unwrap();
        assert!(drain_display_capture().0.is_empty());
        root.set_attribute("data-shot", "plate").unwrap();
        layer.update(&root, Some(&graph), Some(1)).unwrap();
        assert!(
            drain_display_capture()
                .0
                .iter()
                .any(|event| event.kind == "figure-hidden")
        );
        root.remove();
    }

    #[test]
    fn unloaded_scene_plate_uses_a_neutral_backdrop_instead_of_the_empty_room() {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        let style = document.create_element("style").unwrap();
        style.set_text_content(Some(include_str!("live_plate.css")));
        document.body().unwrap().append_child(&style).unwrap();
        let root = document.create_element("div").unwrap();
        root.set_attribute("class", "tv-stage").unwrap();
        root.set_attribute("data-tv-mode", "scene").unwrap();
        root.set_attribute("data-plate-state", "pending").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let set = document.create_element("div").unwrap();
        set.set_attribute("class", "tv-scene-set").unwrap();
        set.set_attribute(
            "style",
            "background-image:url('/empty-shop-placeholder.jpg')",
        )
        .unwrap();
        root.append_child(&set).unwrap();
        let computed = js_sys::Reflect::get(&window, &"getComputedStyle".into())
            .unwrap()
            .dyn_into::<js_sys::Function>()
            .unwrap();
        for state in ["pending", "ready", "failed"] {
            root.set_attribute("data-plate-state", state).unwrap();
            root.remove_attribute("data-plate-loaded").unwrap();
            let css = computed.call1(&window, &set).unwrap();
            let background = js_sys::Reflect::get(&css, &"backgroundImage".into())
                .unwrap()
                .as_string()
                .unwrap();
            assert!(background.contains("radial-gradient"));
            assert!(!background.contains("empty-shop-placeholder"));
        }
        root.remove();
        style.remove();
    }

    #[test]
    fn scene_cast_uses_the_graph_or_fallback_and_recovers_after_battle_and_plate_failure() {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        let style = document.create_element("style").unwrap();
        style.set_text_content(Some(&format!(
            "{}\n{}\n.tv-stage *{{transition:none!important;animation:none!important}}",
            include_str!("live_stage.css"),
            include_str!("live_plate.css"),
        )));
        document.body().unwrap().append_child(&style).unwrap();
        let root = document.create_element("div").unwrap();
        root.set_attribute("class", "tv-stage").unwrap();
        root.set_attribute("data-tv-mode", "scene").unwrap();
        root.set_attribute("data-battle-state", "absent").unwrap();
        let mut conductor = crate::stage_conductor::Conductor::<()>::new();
        conductor.set_battlefield_expected(true);
        root.set_attribute(
            "data-battlefield-live",
            if conductor.battlefield_live() {
                "true"
            } else {
                "false"
            },
        )
        .unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let layer = StageLayer::mount(&document, &root).unwrap();
        layer.update(&root, Some(&graph()), Some(1)).unwrap();
        let actors = root.query_selector(".tv-scene-actors").unwrap().unwrap();
        let legacy_body = document.create_element("div").unwrap();
        legacy_body.set_attribute("class", "tv-fig-body").unwrap();
        root.append_child(&legacy_body).unwrap();
        let computed = js_sys::Reflect::get(&window, &"getComputedStyle".into())
            .unwrap()
            .dyn_into::<js_sys::Function>()
            .unwrap();
        for (shot, plate_state, graph_visibility) in [
            ("stage", "none", "visible"),
            ("plate", "ready", "hidden"),
            ("stage", "failed", "visible"),
        ] {
            root.set_attribute("data-shot", shot).unwrap();
            // The failed plate returns to the graph; the legacy cast stays hidden so it
            // never doubles the actors already shown by that graph.
            root.set_attribute("data-plate-state", plate_state).unwrap();
            for (body, expected) in [(&actors, graph_visibility), (&legacy_body, "hidden")] {
                let css = computed.call1(&window, body).unwrap();
                assert_eq!(
                    js_sys::Reflect::get(&css, &"visibility".into())
                        .unwrap()
                        .as_string()
                        .as_deref(),
                    Some(expected)
                );
            }
        }
        // A flat combat fallback must not inherit the scene graph's suppression rule.
        root.set_attribute("data-tv-mode", "combat").unwrap();
        root.set_attribute("data-battle-state", "turn-start")
            .unwrap();
        for (body, expected) in [(&actors, "hidden"), (&legacy_body, "visible")] {
            let css = computed.call1(&window, body).unwrap();
            assert_eq!(
                js_sys::Reflect::get(&css, &"visibility".into())
                    .unwrap()
                    .as_string()
                    .as_deref(),
                Some(expected)
            );
        }
        // Returning to dialogue without a graph reveals the retained figure fallback.
        root.set_attribute("data-tv-mode", "scene").unwrap();
        root.set_attribute("data-battle-state", "absent").unwrap();
        let css = computed.call1(&window, &actors).unwrap();
        assert_eq!(
            js_sys::Reflect::get(&css, &"visibility".into())
                .unwrap()
                .as_string()
                .as_deref(),
            Some("visible")
        );
        layer.update(&root, None, None).unwrap();
        let css = computed.call1(&window, &legacy_body).unwrap();
        assert_eq!(
            js_sys::Reflect::get(&css, &"visibility".into())
                .unwrap()
                .as_string()
                .as_deref(),
            Some("visible")
        );
        root.remove();
        style.remove();
    }

    #[test]
    fn speech_revision_retains_the_loaded_plate_and_the_same_figure_node() {
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        let layer = StageLayer::mount(&document, &root).unwrap();
        let mut graph = graph();
        layer.update(&root, Some(&graph), Some(1)).unwrap();
        root.set_attribute("data-plate-loaded", "").unwrap();
        let actor = root.query_selector(".tv-scene-actor").unwrap().unwrap();
        graph.revision = 2;
        graph.actors[0].speaking = true;
        graph.actors[0].expression = "fond".into();
        layer.update(&root, Some(&graph), Some(1)).unwrap();
        let retained = root.query_selector(".tv-scene-actor").unwrap().unwrap();
        assert!(actor.is_same_node(Some(&retained)));
        assert!(root.has_attribute("data-plate-loaded"));
        assert_eq!(
            root.get_attribute("data-stage-frame-revision").as_deref(),
            Some("1")
        );
    }

    #[test]
    fn speaking_and_camera_updates_apply_even_when_the_bake_revision_is_unchanged() {
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        let layer = StageLayer::mount(&document, &root).unwrap();
        let mut graph = graph();
        layer.update(&root, Some(&graph), Some(1)).unwrap();
        graph.camera = Camera::Speaker;
        graph.actors[0].speaking = true;
        layer.update(&root, Some(&graph), Some(1)).unwrap();
        assert_eq!(
            layer.root.get_attribute("data-camera").as_deref(),
            Some("speaker")
        );
        let actor = root.query_selector(".tv-scene-actor").unwrap().unwrap();
        assert_eq!(
            actor.get_attribute("data-speaking").as_deref(),
            Some("true")
        );
    }

    #[test]
    fn a_repeated_graph_does_not_replace_its_change_log_or_reload_actor_art() {
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        let layer = StageLayer::mount(&document, &root).unwrap();
        let graph = graph();
        layer.update(&root, Some(&graph), Some(1)).unwrap();
        let changes = root.get_attribute("data-stage-log");
        let actor = root.query_selector(".tv-scene-actor").unwrap().unwrap();
        actor.set_attribute("data-active", "0").unwrap();
        layer.update(&root, Some(&graph), Some(1)).unwrap();
        assert_eq!(root.get_attribute("data-stage-log"), changes);
        assert_eq!(actor.get_attribute("data-active").as_deref(), Some("0"));
    }
}
