//! The display's composited stage: the characters standing in the scene painting.
//!
//! The painting is the back layer; each character is an independent element above it, placed on
//! a depth axis so it stands in the picture rather than on it: near figures (the heroes, knees-up
//! and cropped by the screen edge), mid figures (the speaker and the cast) and far figures
//! (those who stand back). Depth sets size, height on the floor band, dimming and haze; mid and
//! far figures stand on a contact shadow with their feet shown, using the full-body art where it
//! exists. Everything shown comes from the accepted view's cast; the effects (step forward,
//! shake, lunge, heal shimmer, floating numbers) are CSS animations on the figure elements,
//! started by `live_motion` and `update` and never owning game state.
//!
//! Names and health live in a compact HUD at the screen edge, not under the figures; a name
//! appears on a character's speech bubble. Speech bubbles stand BESIDE the speaker's head: each
//! is tried on both sides, then narrower, until it overlaps no other head, the roll panel or the
//! screen edge.
//!
//! The cast is arranged by what the scene is doing, and the stylesheet draws each arrangement
//! from the `data-arrange` flag set here:
//! - `stage`: a non-player character is speaking. That figure stands mid-depth near the centre,
//!   lit; the others stand back (far) and dim.
//! - `party`: nobody is speaking; the cast stands mid-depth between the heroes.
//! - `combat`: the fight and its aftermath, drawn flat as before.
//!
//! The persistent elements are updated in place. Animations that must restart (a second hit,
//! a new line) alternate between two keyframe names through a `data-fx-tick` flag, so no
//! layout is read and no timer runs.
use super::live_health;
use super::live_sfx::{self, Cue};
use super::*;
use crate::health_cues::Blow;
use crate::{
    plate_layout::{self, PlateFrame, Rect, Side},
    presentation::{ActorSlot, MAX_NPCS, PlateGeometry, SceneFamily, StageActor},
    stage_art,
    stage_conductor::IntentKind,
    stage_depth,
    stage_machines::{ActorState, LineState},
};
use std::{cell::Cell, time::Duration};
use web_sys::Event;

const GENERATED: &str = live_tv::GENERATED;
const HEROES: usize = 2;
const SLOTS: usize = HEROES + MAX_NPCS;
/// Two hero cards and the foe's.
const HUD_CARDS: usize = HEROES + 1;
const FOE_CARD: usize = HEROES;
/// Hit-point fraction, in percent, under which a figure breathes heavier and shows a vignette.
const LOW_HEALTH_PERCENT: u32 = 35;
/// An action call-out stays this long, plus a little for long text.
const ACTION_HOLD_MS: u64 = 5500;
const HOLD_PER_EXTRA_CHARACTER_MS: u64 = 40;
const MAXIMUM_ACTION_HOLD_MS: u64 = 8500;
/// A hero's typed line stays this long before it fades; a non-player line stays until replaced.
/// It outlasts a slow model-written reply, which the hero's line waits beside.
const TYPED_HOLD_MS: u64 = 20_000;
const FADE_OUT_MS: u64 = 500;
const BUBBLE_OUT_ANIMATION: &str = "df-bubble-out";
const FLOAT_ANIMATION: &str = "df-float-up";
// PLAY-L01: the animation that fades a replaced cut-out away (see `live_imagery.css`).
const GHOST_ANIMATION: &str = "df-sprite-ghost";

/// Which part of the display the figures are standing in.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum StageMode {
    Scene,
    Combat,
    Aftermath,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Fx {
    Hit,
    Dodge,
    Lunge,
    Heal,
}

impl Fx {
    fn name(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Dodge => "dodge",
            Self::Lunge => "lunge",
            Self::Heal => "heal",
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum BubbleKind {
    /// A non-player character's line: stays attributed until replaced.
    Speech,
    /// A hero's typed line: attributed for a while, then fades.
    Typed,
    /// A call-out of what a character just did: fades after a few seconds.
    Action,
}

impl BubbleKind {
    fn name(self) -> &'static str {
        match self {
            Self::Speech => "speech",
            Self::Typed => "typed",
            Self::Action => "action",
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum FloatKind {
    Damage,
    Critical,
    Miss,
    Heal,
}

impl FloatKind {
    fn name(self) -> &'static str {
        match self {
            Self::Damage => "damage",
            Self::Critical => "critical",
            Self::Miss => "miss",
            Self::Heal => "heal",
        }
    }
}

fn index(slot: ActorSlot) -> Option<usize> {
    match slot {
        ActorSlot::Hero(position) if position < HEROES => Some(position),
        ActorSlot::Npc(position) if position < MAX_NPCS => Some(HEROES + position),
        ActorSlot::Hero(_) | ActorSlot::Npc(_) => None,
    }
}

/// The stage position of a standing place: heroes first, then the non-player figures.
pub(super) fn slot_index(slot: ActorSlot) -> Option<usize> {
    index(slot)
}

/// The standing place at a stage position (the inverse of [`slot_index`]).
pub(super) fn slot_of_index(position: usize) -> ActorSlot {
    slot_at(position)
}

fn slot_at(position: usize) -> ActorSlot {
    if position < HEROES {
        ActorSlot::Hero(position)
    } else {
        ActorSlot::Npc(position - HEROES)
    }
}

fn slot_name(position: usize) -> String {
    if position < HEROES {
        format!("hero-{}", position + 1)
    } else {
        format!("npc-{}", position - HEROES)
    }
}

/// Which side of the screen a standing place is on, for the direction of a lunge or a dodge.
fn slot_position(position: usize) -> f32 {
    if position == 0 {
        0.15
    } else if position == 1 {
        0.87
    } else {
        0.68
    }
}

/// How far from the camera a figure stands.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Depth {
    /// The heroes in the foreground: large, cropped by the screen edge.
    Near,
    /// The speaker, or the cast when nobody speaks.
    Mid,
    /// Those who stand back while another speaks.
    Far,
    /// The fight and its aftermath, drawn flat.
    Flat,
}

impl Depth {
    fn name(self) -> &'static str {
        match self {
            Self::Near => "near",
            Self::Mid => "mid",
            Self::Far => "far",
            Self::Flat => "flat",
        }
    }

    /// Whether the figure's feet are on screen, so the full-body art is the right one.
    fn shows_feet(self) -> bool {
        !matches!(self, Self::Near)
    }
}

/// The art a figure tries in order: the full body, the cut-out, the 2:3 portrait, the small
/// portrait.
struct ArtState {
    candidates: Vec<(&'static str, String)>,
    next: usize,
}

struct Figure {
    root: Element,
    body: Element,
    image: Element,
    bubbles: Element,
    effects: Element,
    art: Rc<RefCell<ArtState>>,
    error: Closure<dyn FnMut(Event)>,
    name_text: RefCell<String>,
}

/// One entry of the HUD: the compact name and health plate of a hero, or of the foe.
struct HudCard {
    root: Element,
    crest: Element,
    name: Element,
    role: Element,
    hit_points: Element,
    hit_points_text: Element,
    last_hit_points: Cell<Option<u32>>,
    // PLAY-H01: the bar's maximum, for the share of it a blow takes.
    maximum: Cell<u32>,
}

/// What a line's bubble shows.
pub(super) struct LineBubble<'a> {
    pub(super) kind: BubbleKind,
    pub(super) speaker: &'a str,
    pub(super) text: &'a str,
    pub(super) voiced: bool,
}

pub(super) struct Figures {
    root: Element,
    /// PLAY-V09: where the bubbles stand while a scene plate covers the cut-outs.
    plate_bubbles: Element,
    /// PLAY-V09: the plate's layout while it is on screen.
    plate: RefCell<Option<PlateGeometry>>,
    figures: Vec<Figure>,
    hud: Vec<HudCard>,
    animation_end: Closure<dyn FnMut(Event)>,
    scene: RefCell<String>,
    document: Document,
}

fn art_candidates(sprite: &str, portrait: &str, full_body: bool) -> Vec<(&'static str, String)> {
    let mut candidates = Vec::new();
    let sprite_stem = stage_art::stem_of(sprite);
    if full_body && let Some(stem) = stage_depth::full_body_stem(sprite_stem) {
        candidates.push(("figure", format!("{GENERATED}/{stem}")));
    }
    if !sprite.is_empty() {
        candidates.push(("sprite", sprite.to_owned()));
    }
    let stem = stage_art::stem_of(portrait);
    if stem.starts_with("portrait-") {
        candidates.push(("portrait", format!("{GENERATED}/{stem}-full")));
    }
    if !portrait.is_empty() {
        candidates.push(("card", portrait.to_owned()));
    }
    candidates
}

fn use_candidate(figure: &Element, image: &Element, kind: &str, url: &str) -> Result<(), JsValue> {
    figure.set_attribute("data-art", kind)?;
    image.set_attribute("src", url)
}

/// How the cast stands for this view.
fn arrangement(cast: &[StageActor], mode: StageMode) -> &'static str {
    let centre_stage = cast
        .iter()
        .any(|actor| matches!(actor.slot, ActorSlot::Npc(_)) && actor.speaking);
    match mode {
        StageMode::Combat | StageMode::Aftermath => "combat",
        StageMode::Scene if centre_stage => "stage",
        StageMode::Scene => "party",
    }
}

/// The element's box on screen as (left, top, right, bottom). Read through the DOM method by
/// name so the geometry interface needs no extra browser binding.
fn rectangle(element: &Element) -> (f64, f64, f64, f64) {
    let target: &JsValue = element.as_ref();
    let rect = js_sys::Reflect::get(target, &JsValue::from_str("getBoundingClientRect"))
        .ok()
        .and_then(|method| method.dyn_into::<js_sys::Function>().ok())
        .and_then(|method| method.call0(target).ok());
    let side = |name: &str| {
        rect.as_ref()
            .and_then(|rect| js_sys::Reflect::get(rect, &JsValue::from_str(name)).ok())
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0)
    };
    (side("left"), side("top"), side("right"), side("bottom"))
}

fn overlaps(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

impl Figures {
    pub(super) fn mount(document: &Document, parent: &Element) -> Result<Self, JsValue> {
        let root = node(document, parent, "div", "tv-figures", "")?;
        root.set_attribute("aria-hidden", "true")?;
        let mut figures = Vec::with_capacity(SLOTS);
        for position in 0..SLOTS {
            let figure = node(document, &root, "figure", "tv-fig", "")?;
            figure.set_attribute("data-slot", &slot_name(position))?;
            figure.set_attribute("data-present", "false")?;
            figure.set_attribute("data-actor-state", ActorState::Offstage.name())?;
            let bubbles = node(document, &figure, "div", "tv-fig-bubbles", "")?;
            let body = node(document, &figure, "div", "tv-fig-body", "")?;
            node(document, &body, "div", "tv-fig-shadow", "")?;
            let image = node(document, &body, "img", "tv-fig-sprite", "")?;
            image.set_attribute("alt", "")?;
            image.set_attribute("decoding", "async")?;
            let effects = node(document, &body, "div", "tv-fig-fx", "")?;
            let art = Rc::new(RefCell::new(ArtState {
                candidates: Vec::new(),
                next: 0,
            }));
            let error = {
                let (figure, image, art) = (figure.clone(), image.clone(), Rc::clone(&art));
                Closure::wrap(Box::new(move |_: Event| {
                    let mut state = art.borrow_mut();
                    match state.candidates.get(state.next).cloned() {
                        Some((kind, url)) => {
                            state.next += 1;
                            // A refused attribute write only leaves the previous art shown.
                            let _applied = use_candidate(&figure, &image, kind, &url);
                        }
                        None => {
                            let _marked = figure.set_attribute("data-art", "none");
                        }
                    }
                }) as Box<dyn FnMut(Event)>)
            };
            image.add_event_listener_with_callback("error", error.as_ref().unchecked_ref())?;
            figures.push(Figure {
                root: figure,
                body,
                image,
                bubbles,
                effects,
                art,
                error,
                name_text: RefCell::new(String::new()),
            });
        }
        // Thin air in front of the far and mid figures, behind the near ones.
        node(document, &root, "div", "tv-mist", "")?;
        let plate_bubbles = node(document, &root, "div", "tv-plate-bubbles", "")?;
        let hud_root = node(document, &root, "div", "tv-hud", "")?;
        let mut hud = Vec::with_capacity(HUD_CARDS);
        for position in 0..HUD_CARDS {
            let card = node(document, &hud_root, "div", "tv-hud-card", "")?;
            card.set_attribute(
                "data-slot",
                &if position == FOE_CARD {
                    "foe".to_owned()
                } else {
                    slot_name(position)
                },
            )?;
            card.set_attribute("data-present", "false")?;
            let crest = node(document, &card, "div", "tv-hud-crest", "")?;
            let plate = node(document, &card, "div", "tv-hud-plate", "")?;
            let name = node(document, &plate, "div", "tv-hud-name", "")?;
            let role = node(document, &plate, "div", "tv-hud-role", "")?;
            let hit_points = node(document, &plate, "div", "tv-fig-hp", "")?;
            node(document, &hit_points, "span", "tv-fig-hp-fill", "")?;
            live_health::mount_bar(document, &hit_points)?;
            let hit_points_text = node(document, &hit_points, "span", "tv-fig-hp-text", "")?;
            hud.push(HudCard {
                root: card,
                crest,
                name,
                role,
                hit_points,
                hit_points_text,
                last_hit_points: Cell::new(None),
                maximum: Cell::new(0),
            });
        }
        let animation_end = Closure::wrap(Box::new(move |event: Event| {
            let finished = js_sys::Reflect::get(&event, &JsValue::from_str("animationName"))
                .ok()
                .and_then(|name| name.as_string())
                .is_some_and(|name| {
                    name == BUBBLE_OUT_ANIMATION
                        || name == FLOAT_ANIMATION
                        || name == GHOST_ANIMATION
                });
            if finished
                && let Some(element) = event.target().and_then(|t| t.dyn_into::<Element>().ok())
            {
                element.remove();
            }
        }) as Box<dyn FnMut(Event)>);
        root.add_event_listener_with_callback(
            "animationend",
            animation_end.as_ref().unchecked_ref(),
        )?;
        Ok(Self {
            root,
            plate_bubbles,
            plate: RefCell::new(None),
            figures,
            hud,
            animation_end,
            scene: RefCell::new(String::new()),
            document: document.clone(),
        })
    }

    pub(super) fn dispose(&self) {
        // Listeners die with their detached elements; removing them is hygiene, and a
        // refusal leaves nothing observable.
        let _removed = self.root.remove_event_listener_with_callback(
            "animationend",
            self.animation_end.as_ref().unchecked_ref(),
        );
        for figure in &self.figures {
            let _removed = figure.image.remove_event_listener_with_callback(
                "error",
                figure.error.as_ref().unchecked_ref(),
            );
        }
    }

    /// Shows the cast of the accepted view, arranged by what the scene is doing. The server's
    /// cast already keeps every character of the scene on stage.
    pub(super) fn update(
        &self,
        cast: &[StageActor],
        scene: &str,
        scene_asset: &str,
        family: SceneFamily,
        mode: StageMode,
    ) -> Result<(), JsValue> {
        let arrange = arrangement(cast, mode);
        self.root.set_attribute("data-grade", family.name())?;
        self.root.set_attribute(
            "data-mode",
            match mode {
                StageMode::Scene => "scene",
                StageMode::Combat => "combat",
                StageMode::Aftermath => "aftermath",
            },
        )?;
        self.root.set_attribute("data-arrange", arrange)?;
        // The stylesheet and the stage's other elements follow the arrangement too.
        if let Some(parent) = self.root.parent_element() {
            parent.set_attribute("data-arrange", arrange)?;
        }
        let staging = stage_depth::staging_for(scene_asset);
        self.root.set_attribute(
            "style",
            &format!(
                "--feet-mid:{}vh;--feet-far:{}vh;--key:{}",
                staging.feet_mid,
                staging.feet_far,
                if staging.key_from_left { 1 } else { -1 }
            ),
        )?;
        flag(&self.root, "data-wet", staging.wet)?;
        if *self.scene.borrow() != scene {
            *self.scene.borrow_mut() = scene.to_owned();
            self.clear_bubbles()?;
        }
        // Non-player figures that do not hold the stage stand back, in the cast's order.
        let stood_back = cast
            .iter()
            .filter(|actor| {
                matches!(actor.slot, ActorSlot::Npc(_))
                    && !Self::holds_stage(actor, mode)
                    && !Self::is_foe(actor, mode)
            })
            .count();
        self.root
            .set_attribute("data-back", &stood_back.to_string())?;
        let mut rank = 0;
        let mut foe_shown = false;
        for (position, figure) in self.figures.iter().enumerate() {
            let actor = cast
                .iter()
                .find(|actor| index(actor.slot) == Some(position));
            match actor {
                Some(actor) => {
                    let role = if matches!(actor.slot, ActorSlot::Hero(_)) {
                        None
                    } else if Self::holds_stage(actor, mode) {
                        Some("focus")
                    } else if Self::is_foe(actor, mode) {
                        Some("foe")
                    } else {
                        Some("back")
                    };
                    match role {
                        Some(role) => figure.root.set_attribute("data-role", role)?,
                        None => figure.root.remove_attribute("data-role")?,
                    }
                    if role == Some("back") {
                        figure.root.set_attribute("data-rank", &rank.to_string())?;
                        rank += 1;
                    } else {
                        figure.root.remove_attribute("data-rank")?;
                    }
                    let depth = match (arrange, role) {
                        ("combat", _) => Depth::Flat,
                        (_, None) => Depth::Near,
                        ("stage", Some("back")) => Depth::Far,
                        _ => Depth::Mid,
                    };
                    figure.root.set_attribute("data-depth", depth.name())?;
                    self.apply(figure, actor, depth)?;
                    if role == Some("foe") && !foe_shown {
                        foe_shown = true;
                        self.show_card(FOE_CARD, actor, mode, figure)?;
                    }
                }
                None => {
                    figure.root.set_attribute("data-present", "false")?;
                }
            }
        }
        if !foe_shown && let Some(card) = self.hud.get(FOE_CARD) {
            card.root.set_attribute("data-present", "false")?;
            card.last_hit_points.set(None);
        }
        for position in 0..HEROES {
            let hero = cast
                .iter()
                .find(|actor| actor.slot == ActorSlot::Hero(position));
            match (hero, self.figures.get(position)) {
                (Some(hero), Some(figure)) => self.show_card(position, hero, mode, figure)?,
                _ => {
                    if let Some(card) = self.hud.get(position) {
                        card.root.set_attribute("data-present", "false")?;
                        card.last_hit_points.set(None);
                    }
                }
            }
        }
        Ok(())
    }

    /// The one non-player character who speaks in a scene stands centre stage.
    fn holds_stage(actor: &StageActor, mode: StageMode) -> bool {
        mode == StageMode::Scene && actor.speaking && matches!(actor.slot, ActorSlot::Npc(_))
    }

    /// In a fight the foe is the non-player character who has hit points; once it is over,
    /// the one who speaks stands where the foe stood.
    fn is_foe(actor: &StageActor, mode: StageMode) -> bool {
        matches!(actor.slot, ActorSlot::Npc(_))
            && match mode {
                StageMode::Scene => false,
                StageMode::Combat => actor.hit_points.is_some(),
                StageMode::Aftermath => actor.hit_points.is_some() || actor.speaking,
            }
    }

    fn apply(&self, figure: &Figure, actor: &StageActor, depth: Depth) -> Result<(), JsValue> {
        let root = &figure.root;
        root.set_attribute("data-present", "true")?;
        flag(root, "data-down", actor.down)?;
        if *figure.name_text.borrow() != actor.name {
            *figure.name_text.borrow_mut() = actor.name.clone();
        }
        let low = actor.hit_points.is_some_and(|(current, maximum)| {
            u64::from(current.min(maximum)) * 100 / u64::from(maximum.max(1))
                < u64::from(LOW_HEALTH_PERCENT)
                && current > 0
        });
        flag(root, "data-low", low)?;
        let tier = live_health::display_tier(actor.hit_points, actor.down);
        live_health::set_tier(root, tier)?;
        live_health::sync_tremor(&figure.body, tier)?;
        self.choose_art(figure, actor, depth)
    }

    /// Fills the HUD card of a hero or the foe, healing visibly when hit points rise.
    fn show_card(
        &self,
        card_index: usize,
        actor: &StageActor,
        mode: StageMode,
        figure: &Figure,
    ) -> Result<(), JsValue> {
        let Some(card) = self.hud.get(card_index) else {
            return Ok(());
        };
        card.root.set_attribute("data-present", "true")?;
        flag(&card.root, "data-down", actor.down)?;
        card.name.set_text_content(Some(&actor.name));
        card.role.set_text_content(Some(&actor.role));
        let tier = live_health::display_tier(actor.hit_points, actor.down);
        live_health::set_tier(&card.root, tier)?;
        live_health::sync_tremor(&card.root, tier)?;
        live_health::crest(&card.crest, &actor.portrait, actor.live_sprite.as_deref())?;
        match actor.hit_points {
            Some((current, maximum)) => {
                let percent = u64::from(current.min(maximum)) * 100 / u64::from(maximum.max(1));
                card.hit_points.remove_attribute("hidden")?;
                card.hit_points
                    .set_attribute("style", &format!("--hp:{percent}%"))?;
                card.hit_points_text
                    .set_text_content(Some(&format!("{current} / {maximum} HP")));
                flag(
                    &card.root,
                    "data-low",
                    percent < u64::from(LOW_HEALTH_PERCENT) && current > 0,
                )?;
                live_health::bar_changed(
                    &card.hit_points,
                    card.last_hit_points.get(),
                    (current, maximum),
                )?;
                card.maximum.set(maximum);
                if let Some(before) = card.last_hit_points.get()
                    && current > before
                    && matches!(actor.slot, ActorSlot::Hero(_))
                {
                    self.heal(figure, actor, current - before, mode)?;
                }
                card.last_hit_points.set(Some(current));
            }
            None => {
                card.hit_points.set_attribute("hidden", "")?;
                card.root.remove_attribute("data-low")?;
                card.last_hit_points.set(None);
            }
        }
        Ok(())
    }

    /// Restarts the art chain only when the character, the depth's art or the generated
    /// expression changed.
    fn choose_art(&self, figure: &Figure, actor: &StageActor, depth: Depth) -> Result<(), JsValue> {
        let key = format!("{}|{}|{}", actor.sprite, actor.portrait, depth.shows_feet());
        // PLAY-L01: a generated expression variant of the cut-out leads the art chain, in the
        // same slot and with the same framing as the art it replaces.
        let live = actor.live_sprite.as_deref().unwrap_or_default();
        let same_art = figure.root.get_attribute("data-portrait").as_deref() == Some(key.as_str());
        let same_live = figure.root.get_attribute("data-live").unwrap_or_default() == live;
        if same_art && same_live {
            return Ok(());
        }
        if same_art {
            self.fade_out_previous(figure)?;
        }
        figure.root.set_attribute("data-portrait", &key)?;
        if live.is_empty() {
            figure.root.remove_attribute("data-live")?;
        } else {
            figure.root.set_attribute("data-live", live)?;
        }
        let mut candidates = art_candidates(&actor.sprite, &actor.portrait, depth.shows_feet());
        if !live.is_empty() {
            let kind = match candidates.first() {
                Some((kind @ ("figure" | "sprite"), _)) => *kind,
                _ => "sprite",
            };
            candidates.insert(0, (kind, live.to_owned()));
        }
        if candidates.is_empty() {
            return figure.root.set_attribute("data-art", "none");
        }
        let (kind, url) = candidates.remove(0);
        {
            let mut state = figure.art.borrow_mut();
            state.candidates = candidates;
            state.next = 0;
        }
        use_candidate(&figure.root, &figure.image, kind, &url)
    }

    /// PLAY-L01: leaves a copy of the figure's current picture above it, to fade out while the new
    /// expression shows beneath. One copy at most; it removes itself when its animation ends.
    fn fade_out_previous(&self, figure: &Figure) -> Result<(), JsValue> {
        let Some(body) = figure.image.parent_element() else {
            return Ok(());
        };
        while let Some(old) = body.query_selector(".tv-fig-ghost")? {
            old.remove();
        }
        let ghost = figure
            .image
            .clone_node_with_deep(false)?
            .dyn_into::<Element>()?;
        ghost.set_attribute("class", "tv-fig-sprite tv-fig-ghost")?;
        body.insert_before(&ghost, figure.image.next_sibling().as_ref())?;
        Ok(())
    }

    fn heal(
        &self,
        figure: &Figure,
        actor: &StageActor,
        amount: u32,
        mode: StageMode,
    ) -> Result<(), JsValue> {
        self.start_fx(figure, Fx::Heal, Duration::ZERO, 1)?;
        self.float_on(
            figure,
            &format!("+{amount}"),
            FloatKind::Heal,
            Duration::ZERO,
        )?;
        let text = match mode {
            StageMode::Combat => "catches a second wind",
            StageMode::Aftermath | StageMode::Scene => "rests and recovers",
        };
        self.bubble_on(
            figure,
            BubbleKind::Action,
            &actor.name,
            text,
            Duration::ZERO,
            false,
        )?;
        live_sfx::play(Cue::HealChime, Duration::ZERO);
        Ok(())
    }

    /// A blow that dropped its target is shown before the view that says so is drawn.
    pub(super) fn force_down(&self, slot: ActorSlot) -> Result<(), JsValue> {
        if let Some(figure) = self.figure(slot) {
            figure.root.set_attribute("data-down", "")?;
        }
        let card = match slot {
            ActorSlot::Hero(position) => self.hud.get(position),
            ActorSlot::Npc(_) => self.hud.get(FOE_CARD),
        };
        if let Some(card) = card {
            card.hit_points.set_attribute("style", "--hp:0%")?;
        }
        // PLAY-H01: the target slumps, and the whole stage gives a light shake.
        self.health_blow(slot, 0, false, true)
    }

    /// PLAY-H01: a blow of `damage` hit points landed on the character at `slot`: the figure and
    /// its card shake by the share of the bar it took (sharper for a critical hit, a slump for a
    /// drop to 0), and the whole stage shakes lightly for a critical hit or a drop.
    pub(super) fn health_blow(
        &self,
        slot: ActorSlot,
        damage: u32,
        critical: bool,
        dropped: bool,
    ) -> Result<(), JsValue> {
        let Some(figure) = self.figure(slot) else {
            return Ok(());
        };
        let card = match slot {
            ActorSlot::Hero(position) => self.hud.get(position),
            ActorSlot::Npc(_) => self.hud.get(FOE_CARD).filter(|card| {
                card.name.text_content().as_deref() == Some(figure.name_text.borrow().as_str())
            }),
        };
        let Some(stage) = self.root.parent_element() else {
            return Ok(());
        };
        let blow = Blow {
            lost: damage,
            maximum: card.map_or(0, |card| card.maximum.get()),
            critical,
            dropped,
        };
        let mut parts = vec![&figure.body];
        parts.extend(card.map(|card| &card.root));
        live_health::display_blow(&stage, &parts, blow)
    }

    fn figure(&self, slot: ActorSlot) -> Option<&Figure> {
        index(slot).and_then(|position| self.figures.get(position))
    }

    fn present(figure: &Figure) -> bool {
        figure.root.get_attribute("data-present").as_deref() == Some("true")
    }

    /// The standing place of the present character called `name`.
    pub(super) fn slot_named(&self, name: &str) -> Option<ActorSlot> {
        self.figures
            .iter()
            .enumerate()
            .find_map(|(position, figure)| {
                (Self::present(figure) && *figure.name_text.borrow() == name)
                    .then(|| slot_at(position))
            })
    }

    /// The sign (+1 right, -1 left) from `from` towards `to` across the screen.
    pub(super) fn direction(from: ActorSlot, to: ActorSlot) -> i32 {
        match (index(from), index(to)) {
            (Some(from), Some(to)) if slot_position(to) < slot_position(from) => -1,
            _ => 1,
        }
    }

    fn start_fx(
        &self,
        figure: &Figure,
        fx: Fx,
        delay: Duration,
        direction: i32,
    ) -> Result<(), JsValue> {
        let tick = if figure.root.get_attribute("data-fx-tick").as_deref() == Some("a") {
            "b"
        } else {
            "a"
        };
        figure.root.set_attribute(
            "style",
            &format!("--fx-delay:{}ms;--fx-dir:{direction}", delay.as_millis()),
        )?;
        figure.root.set_attribute("data-fx", fx.name())?;
        figure.root.set_attribute("data-fx-tick", tick)
    }

    pub(super) fn fx(
        &self,
        slot: ActorSlot,
        fx: Fx,
        delay: Duration,
        direction: i32,
    ) -> Result<(), JsValue> {
        match self.figure(slot) {
            Some(figure) => self.start_fx(figure, fx, delay, direction),
            None => Ok(()),
        }
    }

    fn float_on(
        &self,
        figure: &Figure,
        text: &str,
        kind: FloatKind,
        delay: Duration,
    ) -> Result<(), JsValue> {
        let element = node(&self.document, &figure.effects, "div", "tv-float", text)?;
        element.set_attribute("data-kind", kind.name())?;
        element.set_attribute("style", &format!("--fx-delay:{}ms", delay.as_millis()))
    }

    /// A number that rises from the figure: damage, a miss or healing.
    pub(super) fn float_text(
        &self,
        slot: ActorSlot,
        text: &str,
        kind: FloatKind,
        delay: Duration,
    ) -> Result<(), JsValue> {
        match self.figure(slot) {
            Some(figure) => self.float_on(figure, text, kind, delay),
            None => Ok(()),
        }
    }

    fn bubble_on(
        &self,
        figure: &Figure,
        kind: BubbleKind,
        speaker: &str,
        text: &str,
        appear: Duration,
        voiced: bool,
    ) -> Result<(), JsValue> {
        // The same line again (the audio starting after the director already showed it) stays
        // as it is; otherwise the newest replaces the oldest for the same figure.
        let identity = format!("{}|{}|{speaker}|{text}", kind.name(), voiced);
        if kind == BubbleKind::Speech
            && figure.bubbles.first_element_child().is_some_and(|bubble| {
                bubble.get_attribute("data-line").as_deref() == Some(&identity)
            })
        {
            return Ok(());
        }
        figure.bubbles.set_text_content(None);
        let bubble = node(&self.document, &figure.bubbles, "div", "tv-bubble", "")?;
        bubble.set_attribute("data-kind", kind.name())?;
        bubble.set_attribute("data-line", &identity)?;
        if voiced {
            bubble.set_attribute("data-voice-role", "npc")?;
        }
        let appear_ms = appear.as_millis() as u64;
        let extra = (text.chars().count() as u64).saturating_sub(40) * HOLD_PER_EXTRA_CHARACTER_MS;
        let hold = match kind {
            BubbleKind::Speech => 0,
            BubbleKind::Typed => TYPED_HOLD_MS,
            BubbleKind::Action => (ACTION_HOLD_MS + extra).min(MAXIMUM_ACTION_HOLD_MS),
        };
        bubble.set_attribute(
            "style",
            &format!(
                "--appear:{appear_ms}ms;--hold:{}ms;--fade:{FADE_OUT_MS}ms",
                appear_ms + hold
            ),
        )?;
        let head = node(&self.document, &bubble, "div", "tv-bubble-head", "")?;
        node(&self.document, &head, "span", "tv-bubble-name", speaker)?;
        let wave = node(&self.document, &head, "i", "tv-wave", "")?;
        for _ in 0..3 {
            node(&self.document, &wave, "b", "", "")?;
        }
        node(&self.document, &bubble, "p", "tv-bubble-text", text)?;
        self.place_bubble(figure);
        Ok(())
    }

    /// Stands the bubble beside its speaker's head: each placement is tried in turn (toward the
    /// centre of the screen first), then narrower, until the bubble overlaps no other figure's
    /// head, the roll panel or the screen edge. When none fits, the narrowest stays.
    fn place_bubble(&self, speaker: &Figure) {
        // PLAY-V09: over a scene plate the bubble stands beside the speaker's head in the picture.
        let geometry = self.plate.borrow().clone();
        if let Some(geometry) = geometry {
            self.place_bubble_on_plate(speaker, geometry);
            return;
        }
        let Some(window) = web_sys::window() else {
            return;
        };
        let width = window
            .inner_width()
            .ok()
            .and_then(|value| value.as_f64())
            .unwrap_or(1920.0);
        let height = window
            .inner_height()
            .ok()
            .and_then(|value| value.as_f64())
            .unwrap_or(1080.0);
        let screen = (0.0, 0.0, width, height);
        let mut avoid: Vec<(f64, f64, f64, f64)> = Vec::new();
        for other in &self.figures {
            if other.root.is_same_node(Some(speaker.root.as_ref())) || !Self::present(other) {
                continue;
            }
            let (left, top, right, bottom) = rectangle(&other.body);
            let (span, tall) = (right - left, bottom - top);
            avoid.push((
                left + span * 0.2,
                top,
                right - span * 0.2,
                top + tall * 0.36,
            ));
        }
        if let Ok(Some(panel)) = self.document.query_selector(".combat-roll-panel") {
            avoid.push(rectangle(&panel));
        }
        let inward_first = speaker.root.get_attribute("data-slot").as_deref() == Some("hero-2");
        let sides = if inward_first {
            ["left", "right"]
        } else {
            ["right", "left"]
        };
        for narrow in [false, true] {
            for side in sides {
                let _set = speaker.bubbles.set_attribute("data-side", side);
                flag(&speaker.bubbles, "data-narrow", narrow).ok();
                let placed = rectangle(&speaker.bubbles);
                let inside = placed.0 >= screen.0
                    && placed.2 <= screen.2
                    && placed.1 >= screen.1
                    && placed.3 <= screen.3;
                if inside && !avoid.iter().any(|head| overlaps(placed, *head)) {
                    return;
                }
            }
        }
    }

    /// PLAY-V09: tells the stage that a scene plate with this layout covers the cut-outs (or, with
    /// `None`, that none does). The bubbles then stand in the plate's own layer and are placed
    /// from the layout; without a plate they return to their figures.
    pub(super) fn set_plate(&self, geometry: Option<PlateGeometry>) -> Result<(), JsValue> {
        let before = self.plate.replace(geometry.clone());
        if before == geometry {
            return Ok(());
        }
        for figure in &self.figures {
            let host_is_plate = figure
                .bubbles
                .parent_element()
                .is_some_and(|parent| parent.is_same_node(Some(self.plate_bubbles.as_ref())));
            if geometry.is_some() && !host_is_plate {
                // Moving a bubble in the page restarts its entrance animation: the line being
                // spoken must not pop in again as the plate takes over, so its bubbles skip it.
                let mut bubble = figure.bubbles.first_element_child();
                while let Some(open) = bubble {
                    open.set_attribute("data-instant", "")?;
                    bubble = open.next_element_sibling();
                }
                self.plate_bubbles.append_child(&figure.bubbles)?;
            } else if geometry.is_none() && host_is_plate {
                figure
                    .root
                    .insert_before(&figure.bubbles, figure.root.first_child().as_ref())?;
                figure.bubbles.remove_attribute("style")?;
            }
            if figure.bubbles.has_child_nodes() {
                self.place_bubble(figure);
            }
        }
        Ok(())
    }

    /// PLAY-V09: stands the bubble beside a head in the plate, on the side the server kept calm,
    /// with its tail toward the head; never over the head, off the screen or under the party
    /// strip, the title or the choice chips. A hero's bubble stands beside the hero's edge of the
    /// frame (the plate's own staging of the heroes is not sent), and keeps off the speaker's head.
    fn place_bubble_on_plate(&self, bubble_owner: &Figure, geometry: PlateGeometry) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let dimension = |value: Result<JsValue, JsValue>, fallback: f64| {
            value
                .ok()
                .and_then(|value| value.as_f64())
                .unwrap_or(fallback)
        };
        let (width, height) = (
            dimension(window.inner_width(), 1920.0),
            dimension(window.inner_height(), 1080.0),
        );
        let screen = Rect::new(0.0, 0.0, width, height);
        let frame = PlateFrame { screen };
        let speaker_head = frame.head_box(
            geometry.head_x,
            geometry.head_y,
            geometry.head_width,
            geometry.head_height,
        );
        let slot = bubble_owner
            .root
            .get_attribute("data-slot")
            .unwrap_or_default();
        // A hero's bubble stands beside the hero's head when the server found it, and otherwise
        // under that hero's card in the party strip (the plate's own staging of the heroes is not
        // known then, and a guess would put the bubble over somebody else).
        let hero_index = slot
            .strip_prefix("hero-")
            .and_then(|number| number.parse::<usize>().ok())
            .and_then(|number| number.checked_sub(1));
        let mut on_card = false;
        let (head, side, mut avoid) = match hero_index {
            Some(index) => match geometry.heroes.iter().find(|hero| hero.index == index) {
                Some(hero) => (
                    frame.head_box(hero.head_x, hero.head_y, hero.head_width, hero.head_height),
                    hero.bubble_side,
                    vec![speaker_head],
                ),
                None => {
                    on_card = true;
                    let card = self.hud.get(index).map(|card| rectangle(&card.root));
                    let (left, top, right, bottom) = card.unwrap_or((0.0, 0.0, 1.0, 1.0));
                    (
                        Rect::new(left, top, right, bottom),
                        Side::Right,
                        vec![speaker_head],
                    )
                }
            },
            None => (speaker_head, geometry.bubble_side, Vec::new()),
        };
        let mut interface = Vec::new();
        let mut keep_clear = |element: &Element| {
            let (left, top, right, bottom) = rectangle(element);
            if right > left && bottom > top {
                interface.push(Rect::new(left, top, right, bottom));
            }
        };
        for card in &self.hud {
            if card.root.get_attribute("data-present").as_deref() == Some("true") {
                keep_clear(&card.root);
            }
        }
        for selector in [".tv-headline", ".tv-choices", ".combat-roll-panel"] {
            if let Ok(Some(element)) = self.document.query_selector(selector) {
                keep_clear(&element);
            }
        }
        avoid.extend(interface);
        let unit = (width / 100.0).min(height * 1.7778 / 100.0);
        // The tail starts 1.6 units below the bubble's top and is 1.8 units tall.
        let tail_offset = unit * 2.5;
        let container = &bubble_owner.bubbles;
        let mut placement = None;
        for narrow in [false, true] {
            flag(container, "data-narrow", narrow).ok();
            let size = (
                f64::from(container.client_width()),
                f64::from(container.client_height()),
            );
            placement = if on_card {
                plate_layout::place_bubble_stacked(screen, head, size, &avoid)
            } else {
                plate_layout::place_bubble(screen, head, side, size, tail_offset, &avoid)
                    .or_else(|| plate_layout::place_bubble_stacked(screen, head, size, &avoid))
            };
            if placement.is_some() {
                break;
            }
        }
        if placement.is_none() {
            // No clear place at all (a huge bubble): keep off the head and the screen edge only.
            let size = (
                f64::from(container.client_width()),
                f64::from(container.client_height()),
            );
            placement = plate_layout::place_bubble(screen, head, side, size, tail_offset, &[]);
        }
        let Some(placed) = placement else {
            return;
        };
        let _set = container.set_attribute("data-side", placed.side.name());
        let _set = container.set_attribute(
            "style",
            &format!(
                "left:{:.1}px;top:{:.1}px;right:auto",
                placed.rect.left, placed.rect.top
            ),
        );
    }

    /// Puts a bubble with the speaker's name at the figure standing in `slot`.
    pub(super) fn say(
        &self,
        slot: ActorSlot,
        kind: BubbleKind,
        speaker: &str,
        text: &str,
        appear: Duration,
        voiced: bool,
    ) -> Result<(), JsValue> {
        match self.figure(slot) {
            Some(figure) => self.bubble_on(figure, kind, speaker, text, appear, voiced),
            None => Ok(()),
        }
    }

    /// Sets the actor machine's state on the figure at `position`. The speaking mark and the
    /// stage lighting follow it, never the raw view's speaker field.
    pub(super) fn set_actor_state(
        &self,
        position: usize,
        state: ActorState,
    ) -> Result<(), JsValue> {
        let Some(figure) = self.figures.get(position) else {
            return Ok(());
        };
        figure
            .root
            .set_attribute("data-actor-state", state.name())?;
        flag(&figure.root, "data-speaking", state == ActorState::Speaking)?;
        let lit = self
            .figures
            .iter()
            .any(|other| other.root.has_attribute("data-speaking"));
        flag(&self.root, "data-lit", lit)
    }

    /// Applies one transition of a hero's speech-intent loader: a small loader bubble (dots for
    /// typing, a wave for speaking) on that hero's figure, or on the hero's HUD card during a
    /// fight so it never covers the battlefield. Not game state: the machine alone adds,
    /// closes and removes it.
    pub(super) fn set_intent(
        &self,
        position: usize,
        id: u64,
        state: LineState,
        kind: IntentKind,
        hero: &str,
    ) -> Result<(), JsValue> {
        let (Some(figure), Some(card)) = (self.figures.get(position), self.hud.get(position))
        else {
            return Ok(());
        };
        let combat = self.root.get_attribute("data-arrange").as_deref() == Some("combat");
        let host = if combat { &card.root } else { &figure.bubbles };
        let selector = format!("[data-line-id=\"{id}\"]");
        let existing = host.query_selector(&selector)?;
        let loader = match (existing, state) {
            (Some(loader), _) => loader,
            (None, LineState::Composing | LineState::Submitted) => {
                let loader = node(&self.document, host, "div", "tv-bubble tv-intent", "")?;
                loader.set_attribute("data-kind", "intent")?;
                loader.set_attribute("data-line-id", &id.to_string())?;
                loader.set_attribute("data-managed", "")?;
                loader.set_attribute("data-hero", hero)?;
                loader.set_attribute("aria-hidden", "true")?;
                let dots = node(&self.document, &loader, "span", "tv-intent-dots", "")?;
                for _ in 0..3 {
                    node(&self.document, &dots, "i", "", "")?;
                }
                let wave = node(&self.document, &loader, "i", "tv-wave", "")?;
                for _ in 0..3 {
                    node(&self.document, &wave, "b", "", "")?;
                }
                if !combat {
                    self.place_bubble(figure);
                }
                loader
            }
            (None, _) => return Ok(()),
        };
        let shown = match kind {
            IntentKind::Typing | IntentKind::Submitted => "typing",
            IntentKind::Speaking => "speaking",
        };
        loader.set_attribute("data-intent", shown)?;
        loader.set_attribute("data-line-state", state.name())?;
        flag(
            &loader,
            "data-submitted",
            matches!(kind, IntentKind::Submitted),
        )?;
        let live = matches!(state, LineState::Composing | LineState::Submitted);
        if live {
            figure.root.set_attribute("data-intent", state.name())?;
        } else {
            figure.root.remove_attribute("data-intent")?;
        }
        if state == LineState::Done {
            loader.remove();
        }
        Ok(())
    }

    /// Applies one transition of a line's machine to its bubble at the figure at `position`:
    /// the bubble opens as the line opens, shows the speaking mark while it speaks, closes
    /// through the machine and is removed when the line is done. Returns whether a new bubble
    /// was opened.
    pub(super) fn set_line(
        &self,
        position: usize,
        id: u64,
        state: LineState,
        bubble: &LineBubble<'_>,
    ) -> Result<bool, JsValue> {
        let Some(figure) = self.figures.get(position) else {
            return Ok(false);
        };
        let existing = figure
            .bubbles
            .query_selector(&format!("[data-line-id=\"{id}\"]"))?;
        let (element, opened) = match (existing, state) {
            (Some(element), _) => (element, false),
            (None, LineState::Opening | LineState::Speaking | LineState::Lingering) => {
                (self.open_managed(figure, id, state, bubble)?, true)
            }
            (None, _) => return Ok(false),
        };
        element.set_attribute("data-line-state", state.name())?;
        match state {
            LineState::Speaking => {
                element.class_list().add_1("speaking")?;
            }
            LineState::Lingering | LineState::Closing => {
                element.class_list().remove_1("speaking")?;
            }
            LineState::Done => element.remove(),
            _ => {}
        }
        Ok(opened)
    }

    fn open_managed(
        &self,
        figure: &Figure,
        id: u64,
        state: LineState,
        bubble: &LineBubble<'_>,
    ) -> Result<Element, JsValue> {
        figure.bubbles.set_text_content(None);
        let element = node(&self.document, &figure.bubbles, "div", "tv-bubble", "")?;
        element.set_attribute("data-kind", bubble.kind.name())?;
        element.set_attribute("data-line-id", &id.to_string())?;
        element.set_attribute("data-managed", "")?;
        if state == LineState::Lingering {
            // A line already on screen when the page was (a reload): no entrance.
            element.set_attribute("data-instant", "")?;
        }
        if bubble.voiced {
            element.set_attribute("data-voice-role", "npc")?;
        }
        let head = node(&self.document, &element, "div", "tv-bubble-head", "")?;
        node(
            &self.document,
            &head,
            "span",
            "tv-bubble-name",
            bubble.speaker,
        )?;
        let wave = node(&self.document, &head, "i", "tv-wave", "")?;
        for _ in 0..3 {
            node(&self.document, &wave, "b", "", "")?;
        }
        node(&self.document, &element, "p", "tv-bubble-text", bubble.text)?;
        self.place_bubble(figure);
        Ok(element)
    }

    pub(super) fn clear_bubbles(&self) -> Result<(), JsValue> {
        for figure in &self.figures {
            figure.bubbles.set_text_content(None);
            figure.root.remove_attribute("data-thinking")?;
        }
        Ok(())
    }

    /// Shows the small thinking indicator at the figure in `slot` (a speaker writing a reply),
    /// or at none. Showing it where it already is changes nothing, so its animation is not
    /// restarted by every view.
    pub(super) fn set_thinking(&self, slot: Option<ActorSlot>) -> Result<(), JsValue> {
        let target = slot.and_then(|slot| self.figure(slot));
        for figure in &self.figures {
            let wanted = target.is_some_and(|target| target.root.is_same_node(Some(&figure.root)));
            let shown = figure.root.has_attribute("data-thinking");
            if wanted && !shown {
                figure.root.set_attribute("data-thinking", "")?;
                let dots = node(&self.document, &figure.bubbles, "div", "tv-thinking", "")?;
                dots.set_attribute("aria-hidden", "true")?;
                for _ in 0..3 {
                    node(&self.document, &dots, "i", "", "")?;
                }
                self.place_bubble(figure);
            } else if !wanted && shown {
                figure.root.remove_attribute("data-thinking")?;
                if let Some(dots) = figure.bubbles.query_selector(".tv-thinking")? {
                    dots.remove();
                }
            }
        }
        Ok(())
    }
}

fn flag(element: &Element, name: &str, on: bool) -> Result<(), JsValue> {
    if on {
        element.set_attribute(name, "")
    } else {
        element.remove_attribute(name)
    }
}
