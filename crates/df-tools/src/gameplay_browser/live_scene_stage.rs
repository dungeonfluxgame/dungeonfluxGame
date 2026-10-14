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
                    art: actor.art.clone(),
                    fallback_art: actor.fallback_art.clone(),
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
    rejected_frame: Cell<Option<(u64, u64)>>,
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
            rejected_frame: Cell::new(None),
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
            if self.revision.replace(None).is_some() {
                self.scene_key.borrow_mut().clear();
                self.record_change(tv_root, "graph", "scene graph removed from display")?;
            }
            return Ok(());
        };
        validate(graph)?;
        let previous_revision = self.revision.get();
        let previous_scene = self.scene_key.borrow().clone();
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
        self.set.set_attribute(
            "style",
            &format!(
                "background-image:url(\"{}\"),url(\"{}\")",
                css_url(&graph.set.asset),
                css_url(&graph.set.fallback_asset)
            ),
        )?;
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
        if baked_revision == Some(graph.revision) {
            tv_root.set_attribute("data-stage-frame-revision", &graph.revision.to_string())?;
            self.rejected_frame.set(None);
        } else {
            tv_root.remove_attribute("data-stage-frame-revision")?;
            tv_root.remove_attribute("data-plate-loaded")?;
            if let Some(plate) = tv_root.query_selector(".tv-plate")? {
                plate.remove_attribute("data-want")?;
            }
            if let Some(plate_revision) = baked_revision
                && self
                    .rejected_frame
                    .replace(Some((graph.revision, plate_revision)))
                    != Some((graph.revision, plate_revision))
            {
                self.record_change(tv_root, "baked-frame", "plate revision is stale")?;
            }
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
            let actor_present = graph.actors.iter().any(|actor| actor.id == actor_id);
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
            self.record_change(
                tv_root,
                &format!("actor:{actor_name}"),
                if scene_changed {
                    "new scene replaces actor"
                } else {
                    "actor left scene"
                },
            )?;
        }
        for actor in &graph.actors {
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
                    )?);
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
            || actor.art.is_empty()
            || actor.fallback_art.is_empty()
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
) -> Result<ActorLayer, JsValue> {
    let root = node(document, parent, "div", "tv-scene-actor", "")?;
    root.set_attribute("data-actor-id", &actor.id)?;
    root.set_attribute("data-absent", "false")?;
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
                    let _ = root.set_attribute(
                        "data-active",
                        &image.get_attribute("data-i").unwrap_or_default(),
                    );
                    let _ = root.remove_attribute("data-art-unready");
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
                let fallback = image.get_attribute("data-fallback").unwrap_or_default();
                if image.get_attribute("data-source").as_deref() != Some(fallback.as_str()) {
                    let _ = image.set_attribute("data-source", &fallback);
                    let _ = image.set_attribute("src", &fallback);
                } else {
                    let _ = root.set_attribute("data-art-unready", "");
                }
            }) as Box<dyn FnMut(Event)>)
        };
        image.add_event_listener_with_callback("load", load.as_ref().unchecked_ref())?;
        image.add_event_listener_with_callback("error", error.as_ref().unchecked_ref())?;
        images.push(image);
        listeners.push((load, error));
    }
    Ok(ActorLayer {
        root,
        images,
        listeners,
    })
}

fn update_actor(
    layer: &mut ActorLayer,
    actor: &SceneActorView,
    changed_spot: bool,
    graph: &SceneGraphView,
) -> Result<(), JsValue> {
    let root = &layer.root;
    let previous_art = root.get_attribute("data-art-want");
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
    if previous_art.as_deref() != Some(actor.art.as_str())
        || previous_expression.as_deref() != Some(actor.expression.as_str())
        || previous_pose.as_deref() != Some(actor.pose.as_str())
    {
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
        root.set_attribute("data-art-unready", "")?;
    }
    Ok(())
}

fn css_url(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "")
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
