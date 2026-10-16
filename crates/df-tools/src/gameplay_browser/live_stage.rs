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
    plate_layout::{self, PlateFrame, Rect},
    presentation::{ActorSlot, MAX_NPCS, PlateGeometry, SceneFamily, StageActor},
    stage_art,
    stage_conductor::IntentKind,
    stage_depth,
    stage_machines::{ActorState, LineState},
};
use df_observe::presentation::{ComponentPhase, PresentationComponent};
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
const ACTION_HOLD_MS: u64 = 1500;
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
const BUBBLE_UI_OBSTACLES: [&str; 5] = [
    ".tv-headline",
    ".tv-rules",
    ".tv-scene-context",
    ".tv-choices",
    ".combat-roll-panel",
];
const BUBBLE_FIXED_OBSTACLES: [&str; 3] = [".tv-headline", ".tv-rules", ".tv-scene-context"];
const CHECK_ROLL_MS: u32 = 5_000;
const CHECK_FINDING_PADDING_MS: u32 = 1_000;
const CHECK_FINDING_REVEAL_MS: u32 = 200;
const CHECK_FADE_MS: u32 = 500;
const PARTY_HUD_FADE_MS: u32 = 700;
const PARTY_HUD_SCENE: &str = "party_hud";

pub(super) fn line_component_phase(state: LineState) -> ComponentPhase {
    match state {
        LineState::Composing | LineState::Submitted => ComponentPhase::Requested,
        LineState::Queued => ComponentPhase::Queued,
        LineState::Opening => ComponentPhase::Opened,
        LineState::Speaking | LineState::Lingering => ComponentPhase::Readable,
        LineState::Closing | LineState::Cancelled => ComponentPhase::Closing,
        LineState::Done => ComponentPhase::Closing,
    }
}

pub(super) fn line_component_reason(state: LineState) -> &'static str {
    match state {
        LineState::Closing | LineState::Cancelled => "successor",
        LineState::Done => "ended",
        _ => "ready",
    }
}

fn report_removed_presentation_component(element: &Element, reason: &'static str) {
    let component = match element.get_attribute("data-component-type").as_deref() {
        Some("subtitle") => PresentationComponent::Subtitle,
        Some("speech_bubble") => PresentationComponent::SpeechBubble,
        Some("thinking_loader") => PresentationComponent::ThinkingLoader,
        Some("intent_loader") => PresentationComponent::IntentLoader,
        Some("check_card") => PresentationComponent::CheckCard,
        Some("item_card") => PresentationComponent::ItemCard,
        Some("title_card") => PresentationComponent::TitleCard,
        Some("scene_card") => PresentationComponent::SceneCard,
        Some("plate_incoming") => PresentationComponent::PlateIncoming,
        Some("plate_outgoing") => PresentationComponent::PlateOutgoing,
        _ => return,
    };
    let Some(id) = element
        .get_attribute("data-component-id")
        .and_then(|id| id.parse::<u64>().ok())
    else {
        return;
    };
    let scene = element
        .get_attribute("data-component-scene")
        .unwrap_or_default();
    super::presentation_timing::component(
        Some(element),
        component,
        id,
        &scene,
        ComponentPhase::Hidden,
        reason,
        0,
    );
}

#[derive(Clone)]
pub(super) struct CheckDisplay {
    pub(super) sequence: u64,
    pub(super) hero_name: String,
    pub(super) intention: String,
    pub(super) ability_skill: String,
    pub(super) faces: Vec<(u32, bool)>,
    pub(super) roll_mode: i32,
    pub(super) modifier: i32,
    pub(super) total: i32,
    pub(super) succeeded: bool,
    pub(super) finding: String,
}

impl CheckDisplay {
    fn signature(&self) -> String {
        format!(
            "{}|{}|{}|{}|{:?}|{}|{}|{}|{}|{}",
            self.sequence,
            self.hero_name,
            self.intention,
            self.ability_skill,
            self.faces,
            self.roll_mode,
            self.modifier,
            self.total,
            self.succeeded,
            self.finding
        )
    }
}

/// Keep the roll and finding phases explicit. The finding gets its full reading budget after the
/// roll, then a short fade; no part of its budget is spent inside the fade animation.
pub(super) fn check_roll_read_ms() -> u32 {
    CHECK_ROLL_MS.saturating_add(CHECK_FINDING_REVEAL_MS)
}

pub(super) fn check_reveal_ms() -> u32 {
    CHECK_FINDING_REVEAL_MS
}

pub(super) fn check_result_read_ms(finding: &str) -> u32 {
    crate::stage_machines::reading_time_ms(finding).saturating_add(CHECK_FINDING_PADDING_MS)
}

fn check_result_phase_ms(finding: &str) -> (u32, u32, u32) {
    let finding_ms = check_result_read_ms(finding);
    let total_ms = CHECK_ROLL_MS
        .saturating_add(CHECK_FINDING_REVEAL_MS)
        .saturating_add(finding_ms)
        .saturating_add(CHECK_FADE_MS);
    (finding_ms, total_ms, CHECK_ROLL_MS)
}

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

fn should_close_typed_strip(kind: BubbleKind, strip_active: bool) -> bool {
    kind == BubbleKind::Speech && strip_active
}

fn typed_line_anchor(speaker: &str, heroes: &[Option<String>]) -> Option<usize> {
    heroes
        .iter()
        .position(|name| name.as_deref() == Some(speaker))
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
    last_check: Cell<Option<u64>>,
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
    /// The active picture's intrinsic dimensions for its cover crop.
    plate_size: Cell<Option<(f64, f64)>>,
    figures: Vec<Figure>,
    hud: Vec<HudCard>,
    animation_end: Closure<dyn FnMut(Event)>,
    scene: RefCell<String>,
    party_hud_visible: Cell<Option<bool>>,
    /// Monotonic identity for each opening transition. A new identity lets the existing
    /// presentation capture reject a stale Readable callback after an interrupted fade.
    party_hud_transition_id: Cell<u64>,
    document: Document,
}

fn art_candidates(sprite: &str, portrait: &str, full_body: bool) -> Vec<(&'static str, String)> {
    let mut candidates = Vec::new();
    let sprite_stem = stage_art::stem_of(sprite);
    if full_body && let Some(stem) = stage_depth::full_body_stem(sprite_stem) {
        let stem = stage_art::photographic_stem(&stem).unwrap_or(stem.as_str());
        candidates.push(("figure", format!("{GENERATED}/{stem}")));
    }
    if !sprite.is_empty() {
        if let Some(stem) = stage_art::photographic_stem(sprite_stem) {
            candidates.push(("sprite", format!("{GENERATED}/{stem}")));
        } else {
            candidates.push(("sprite", sprite.to_owned()));
        }
    }
    // Once a known photo sibling exists, do not append painted portrait/card fallbacks that can
    // reintroduce the old illustrated asset after a load error.
    if stage_art::photographic_stem(sprite_stem).is_some() {
        return candidates;
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
                last_check: Cell::new(None),
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
            let hit_points_text = node(document, &plate, "span", "tv-fig-hp-text", "")?;
            let hit_points = node(document, &plate, "div", "tv-fig-hp", "")?;
            node(document, &hit_points, "span", "tv-fig-hp-fill", "")?;
            live_health::mount_bar(document, &hit_points)?;
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
                        || name == "df-check-result-out"
                });
            if finished
                && let Some(element) = event.target().and_then(|t| t.dyn_into::<Element>().ok())
            {
                element.remove();
                report_removed_presentation_component(&element, "timeout");
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
            plate_size: Cell::new(None),
            figures,
            hud,
            animation_end,
            scene: RefCell::new(String::new()),
            party_hud_visible: Cell::new(None),
            party_hud_transition_id: Cell::new(0),
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
        self.sync_party_hud(mode)?;
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
            self.clear_bubbles()?;
            for figure in &self.figures {
                figure.last_check.set(None);
            }
            *self.scene.borrow_mut() = scene.to_owned();
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

    fn sync_party_hud(&self, mode: StageMode) -> Result<(), JsValue> {
        let visible = mode == StageMode::Combat;
        let hud = self
            .root
            .query_selector(".tv-hud")?
            .ok_or_else(|| JsValue::from_str("party HUD root absent"))?;
        if self.party_hud_visible.get() == Some(visible) {
            // Repair a reconnect or DOM replacement without restarting the fade for a stable
            // authoritative phase.
            let expected = if visible { "visible" } else { "hidden" };
            if hud.get_attribute("data-party-hud").as_deref() != Some(expected) {
                hud.set_attribute("data-party-hud", expected)?;
            }
            return Ok(());
        }
        self.party_hud_visible.set(Some(visible));
        let id = if visible {
            let id = self.party_hud_transition_id.get().saturating_add(1).max(1);
            self.party_hud_transition_id.set(id);
            id
        } else {
            self.party_hud_transition_id.get().max(1)
        };
        hud.set_attribute("data-party-hud", if visible { "visible" } else { "hidden" })?;
        let phase = if visible {
            ComponentPhase::Opened
        } else {
            ComponentPhase::Closing
        };
        super::presentation_timing::component(
            Some(&hud),
            PresentationComponent::PartyHud,
            id,
            PARTY_HUD_SCENE,
            phase,
            if visible {
                "battle-begin"
            } else {
                "phase-change"
            },
            PARTY_HUD_FADE_MS,
        );
        super::presentation_timing::schedule_component_transition(
            hud,
            PresentationComponent::PartyHud,
            id,
            PARTY_HUD_SCENE.to_owned(),
            if visible {
                ComponentPhase::Readable
            } else {
                ComponentPhase::Hidden
            },
            if visible {
                "battle-begin"
            } else {
                "phase-change"
            },
            PARTY_HUD_FADE_MS,
            PARTY_HUD_FADE_MS,
        );
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
        root.set_attribute("data-actor-id", &actor.actor_id)?;
        figure
            .bubbles
            .set_attribute("data-actor-id", &actor.actor_id)?;
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
        card.root.set_attribute("data-actor-id", &actor.actor_id)?;
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
        let text = healing_action_text(mode, amount);
        self.bubble_on(
            figure,
            BubbleKind::Action,
            &actor.name,
            &text,
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

    fn report_removed_component(&self, element: &Element, position: usize, reason: &'static str) {
        if element.has_attribute("data-component-type") {
            report_removed_presentation_component(element, reason);
            return;
        }
        report_removed_presentation_component(element, reason);
        let component = if element.class_list().contains("tv-check-result") {
            element
                .get_attribute("data-check-signature")
                .map(|signature| {
                    (
                        PresentationComponent::CheckCard,
                        super::presentation_timing::identity(&signature),
                    )
                })
        } else if element.class_list().contains("tv-thinking") {
            Some((PresentationComponent::ThinkingLoader, position as u64 + 1))
        } else if element.class_list().contains("tv-intent") {
            element
                .get_attribute("data-line-id")
                .and_then(|id| id.parse::<u64>().ok())
                .map(|id| (PresentationComponent::IntentLoader, id))
        } else if element.class_list().contains("tv-bubble") {
            element
                .get_attribute("data-line-id")
                .or_else(|| element.get_attribute("data-timing-id"))
                .and_then(|id| id.parse::<u64>().ok())
                .map(|id| {
                    let component = PresentationComponent::SpeechBubble;
                    (component, id)
                })
        } else {
            None
        };
        if let Some((component, id)) = component {
            let scene = self.scene.borrow().clone();
            super::presentation_timing::component(
                Some(element),
                component,
                id,
                &scene,
                ComponentPhase::Hidden,
                reason,
                0,
            );
        }
    }

    fn clear_figure_components(
        &self,
        figure: &Figure,
        position: usize,
        reason: &'static str,
    ) -> Result<(), JsValue> {
        while let Some(element) = figure.bubbles.first_element_child() {
            element.remove();
            self.report_removed_component(&element, position, reason);
        }
        figure.root.remove_attribute("data-thinking")?;
        figure.bubbles.remove_attribute("data-anchor-line")?;
        Ok(())
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

    #[cfg(test)]
    pub(super) fn set_check(&self, check: Option<&CheckDisplay>) -> Result<(), JsValue> {
        self.set_check_mode(check, false, false)
    }

    pub(super) fn begin_check(&self, check: &CheckDisplay, settled: bool) -> Result<(), JsValue> {
        self.set_check_mode(Some(check), true, settled)
    }

    pub(super) fn reveal_check(&self, sequence: u64) -> Result<(), JsValue> {
        for figure in &self.figures {
            if let Some(panel) = figure.bubbles.query_selector(&format!(
                ".tv-check-result[data-check-sequence=\"{sequence}\"]"
            ))? {
                if panel.get_attribute("data-check-phase").as_deref() == Some("roll") {
                    panel.set_attribute("data-check-phase", "result")?;
                    let id = panel
                        .get_attribute("data-component-id")
                        .and_then(|id| id.parse::<u64>().ok())
                        .unwrap_or(sequence);
                    let read_ms = panel
                        .get_attribute("data-check-read-ms")
                        .and_then(|read| read.parse::<u32>().ok())
                        .unwrap_or(0);
                    super::presentation_timing::component(
                        Some(&panel),
                        PresentationComponent::CheckCard,
                        id,
                        &self.scene.borrow(),
                        ComponentPhase::Readable,
                        "ready",
                        read_ms,
                    );
                }
                self.place_bubble(figure);
            }
        }
        Ok(())
    }

    pub(super) fn finish_check(&self, sequence: u64) -> Result<(), JsValue> {
        for figure in &self.figures {
            if let Some(panel) = figure.bubbles.query_selector(&format!(
                ".tv-check-result[data-check-sequence=\"{sequence}\"]"
            ))? {
                panel.remove();
                self.report_removed_component(&panel, 0, "read");
            }
        }
        Ok(())
    }

    pub(super) fn close_check(&self, sequence: u64) -> Result<(), JsValue> {
        for figure in &self.figures {
            if let Some(panel) = figure.bubbles.query_selector(&format!(
                ".tv-check-result[data-check-sequence=\"{sequence}\"]"
            ))? && panel.get_attribute("data-check-phase").as_deref() != Some("closing")
            {
                panel.set_attribute("data-check-phase", "closing")?;
                let id = panel
                    .get_attribute("data-component-id")
                    .and_then(|id| id.parse::<u64>().ok())
                    .unwrap_or(sequence);
                super::presentation_timing::component(
                    Some(&panel),
                    PresentationComponent::CheckCard,
                    id,
                    &self.scene.borrow(),
                    ComponentPhase::Closing,
                    "read",
                    0,
                );
            }
        }
        Ok(())
    }

    fn set_check_mode(
        &self,
        check: Option<&CheckDisplay>,
        managed: bool,
        settled: bool,
    ) -> Result<(), JsValue> {
        let Some(check) = check else {
            for (position, figure) in self.figures.iter().enumerate() {
                while let Some(panel) = figure.bubbles.query_selector(".tv-check-result")? {
                    panel.remove();
                    self.report_removed_component(&panel, position, "successor");
                }
            }
            return Ok(());
        };
        let slot = self
            .slot_named(&check.hero_name)
            .ok_or_else(|| JsValue::from_str("check hero is absent from the scene"))?;
        let figure = self
            .figure(slot)
            .ok_or_else(|| JsValue::from_str("check hero figure is absent"))?;
        let signature = check.signature();
        let component_id = super::presentation_timing::identity(&signature);
        if figure.last_check.get() == Some(check.sequence) {
            if figure.bubbles.query_selector(".tv-check-result")?.is_some() {
                self.place_bubble(figure);
            }
            return Ok(());
        }
        // A rapid second-seat check must not erase the first card while its finding is still
        // readable. Remove only the checked hero's other live bubble; prior result cards remain
        // queued in this container until their timed fade completes.
        let mut child = figure.bubbles.first_element_child();
        while let Some(element) = child {
            child = element.next_element_sibling();
            if !element.class_list().contains("tv-check-result") {
                element.remove();
                self.report_removed_component(
                    &element,
                    index(slot).unwrap_or_default(),
                    "successor",
                );
            }
        }
        let (finding_ms, total_ms, roll_ms) = check_result_phase_ms(&check.finding);
        let scene = self.scene.borrow().clone();
        super::presentation_timing::component(
            None,
            PresentationComponent::CheckCard,
            component_id,
            &scene,
            ComponentPhase::Requested,
            "ready",
            finding_ms,
        );
        let panel = node(
            &self.document,
            &figure.bubbles,
            "aside",
            "tv-check-result",
            "",
        )?;
        panel.set_attribute("data-check-signature", &signature)?;
        panel.set_attribute("data-check-sequence", &check.sequence.to_string())?;
        panel.set_attribute("data-check-read-ms", &finding_ms.to_string())?;
        if managed {
            panel.set_attribute("data-check-managed", "")?;
            panel.set_attribute("data-check-phase", if settled { "result" } else { "roll" })?;
        }
        figure.last_check.set(Some(check.sequence));
        panel.set_attribute("data-component-id", &component_id.to_string())?;
        panel.set_attribute("data-component-scene", &scene)?;
        panel.set_attribute("data-component-type", "check_card")?;
        panel.set_attribute(
            "style",
            &format!(
                "--check-finding-delay:{roll_ms}ms;--check-finding-reveal-duration:{CHECK_FINDING_REVEAL_MS}ms;--check-fade-duration:{CHECK_FADE_MS}ms;--check-close-delay:{}ms",
                total_ms.saturating_sub(CHECK_FADE_MS)
            ),
        )?;
        panel.set_attribute("data-roll-mode", &check.roll_mode.to_string())?;
        panel.set_attribute(
            "data-succeeded",
            if check.succeeded { "true" } else { "false" },
        )?;
        panel.set_attribute("tabindex", "0")?;
        panel.set_attribute("aria-label", &format!("{} check", check.hero_name))?;
        let entry = node(&self.document, &panel, "div", "tv-check-entry", "")?;
        node(
            &self.document,
            &entry,
            "span",
            "tv-check-hero",
            &check.hero_name,
        )?;
        if !check.intention.trim().is_empty() {
            node(
                &self.document,
                &entry,
                "p",
                "tv-check-intention",
                &check.intention,
            )?;
        }
        node(
            &self.document,
            &entry,
            "span",
            "tv-check-skill",
            &check.ability_skill,
        )?;
        let roll = node(&self.document, &panel, "div", "tv-check-roll", "")?;
        let dice = node(&self.document, &roll, "div", "tv-check-dice", "")?;
        node(&self.document, &dice, "span", "tv-check-die-label", "d20")?;
        for (face, kept) in &check.faces {
            let die = node(
                &self.document,
                &dice,
                "span",
                "tv-check-die",
                &face.to_string(),
            )?;
            if *kept {
                die.set_attribute("data-kept", "")?;
                die.set_attribute("aria-label", &format!("{face}, kept"))?;
            }
        }
        let equation = node(&self.document, &roll, "div", "tv-check-equation", "")?;
        let modifier = if check.modifier >= 0 {
            format!("+ {}", check.modifier)
        } else {
            format!("- {}", check.modifier.unsigned_abs())
        };
        node(
            &self.document,
            &equation,
            "span",
            "tv-check-modifier",
            &modifier,
        )?;
        node(&self.document, &equation, "span", "tv-check-operator", "=")?;
        node(
            &self.document,
            &equation,
            "b",
            "tv-check-total",
            &check.total.to_string(),
        )?;
        node(
            &self.document,
            &roll,
            "span",
            "tv-check-mode",
            match check.roll_mode {
                -1 => "Disadvantage",
                1 => "Advantage",
                _ => "D20 roll",
            },
        )?;
        let resolution = node(&self.document, &panel, "div", "tv-check-resolution", "")?;
        node(
            &self.document,
            &resolution,
            "strong",
            "tv-check-outcome",
            if check.succeeded {
                "Success"
            } else {
                "Failure"
            },
        )?;
        if !check.finding.trim().is_empty() {
            node(
                &self.document,
                &resolution,
                "p",
                "tv-check-finding",
                &check.finding,
            )?;
        }
        self.place_bubble(figure);
        super::presentation_timing::component(
            Some(&panel),
            PresentationComponent::CheckCard,
            component_id,
            &scene,
            ComponentPhase::Opened,
            "ready",
            finding_ms,
        );
        if !managed {
            super::presentation_timing::schedule_component_transition(
                panel.clone(),
                PresentationComponent::CheckCard,
                component_id,
                scene.clone(),
                ComponentPhase::Readable,
                "ready",
                finding_ms,
                roll_ms.saturating_add(CHECK_FINDING_REVEAL_MS),
            );
            super::presentation_timing::schedule_component_transition(
                panel.clone(),
                PresentationComponent::CheckCard,
                component_id,
                scene.clone(),
                ComponentPhase::Closing,
                "timeout",
                finding_ms,
                total_ms.saturating_sub(CHECK_FADE_MS),
            );
            super::presentation_timing::schedule_component_transition(
                panel,
                PresentationComponent::CheckCard,
                component_id,
                scene,
                ComponentPhase::Hidden,
                "timeout",
                finding_ms,
                total_ms,
            );
        } else if settled {
            super::presentation_timing::component(
                Some(&panel),
                PresentationComponent::CheckCard,
                component_id,
                &scene,
                ComponentPhase::Readable,
                "settled",
                finding_ms,
            );
        }
        Ok(())
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
        let position = self
            .figures
            .iter()
            .position(|candidate| candidate.root.is_same_node(Some(&figure.root)))
            .unwrap_or_default();
        self.clear_figure_components(figure, position, "successor")?;
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
        if kind == BubbleKind::Action {
            let id = super::presentation_timing::next_component_id();
            bubble.set_attribute("data-timing-id", &id.to_string())?;
            let scene = self.scene.borrow().clone();
            bubble.set_attribute("data-component-id", &id.to_string())?;
            bubble.set_attribute("data-component-scene", &scene)?;
            bubble.set_attribute("data-component-type", "speech_bubble")?;
            let read_ms = crate::stage_machines::reading_time_ms(text);
            super::presentation_timing::component(
                None,
                PresentationComponent::SpeechBubble,
                id,
                &scene,
                ComponentPhase::Requested,
                "ready",
                read_ms,
            );
            super::presentation_timing::component(
                Some(&bubble),
                PresentationComponent::SpeechBubble,
                id,
                &scene,
                ComponentPhase::Opened,
                "ready",
                read_ms,
            );
            super::presentation_timing::schedule_component_transition(
                bubble.clone(),
                PresentationComponent::SpeechBubble,
                id,
                scene.clone(),
                ComponentPhase::Readable,
                "ready",
                read_ms,
                u32::try_from(appear_ms).unwrap_or(u32::MAX),
            );
            super::presentation_timing::schedule_component_transition(
                bubble.clone(),
                PresentationComponent::SpeechBubble,
                id,
                scene.clone(),
                ComponentPhase::Closing,
                "timeout",
                read_ms,
                u32::try_from(appear_ms.saturating_add(hold)).unwrap_or(u32::MAX),
            );
            super::presentation_timing::schedule_component_transition(
                bubble,
                PresentationComponent::SpeechBubble,
                id,
                scene,
                ComponentPhase::Hidden,
                "timeout",
                read_ms,
                u32::try_from(appear_ms.saturating_add(hold).saturating_add(FADE_OUT_MS))
                    .unwrap_or(u32::MAX),
            );
        }
        Ok(())
    }

    /// Stands the bubble beside its speaker's head: each placement is tried in turn (toward the
    /// centre of the screen first), then narrower, until the bubble overlaps no other figure's
    /// head, the roll panel or the screen edge. When none fits, the narrowest stays.
    fn place_bubble(&self, speaker: &Figure) {
        if speaker
            .bubbles
            .query_selector(".tv-check-result[data-check-managed]")
            .ok()
            .flatten()
            .is_some()
        {
            self.place_check_panel(speaker);
            return;
        }
        self.place_bubble_at_actor(speaker);
        if speaker.bubbles.has_attribute("hidden")
            && speaker
                .bubbles
                .query_selector(".tv-check-result")
                .ok()
                .flatten()
                .is_some()
        {
            self.place_check_panel(speaker);
        }
    }

    /// Rules findings stay readable even when this bitmap has no safe character anchor.
    fn place_check_panel(&self, speaker: &Figure) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let (Some(width), Some(height)) = (
            window.inner_width().ok().and_then(|n| n.as_f64()),
            window.inner_height().ok().and_then(|n| n.as_f64()),
        ) else {
            return;
        };
        let host = &speaker.bubbles;
        let _moved = self.plate_bubbles.append_child(host);
        let _shown = host.remove_attribute("hidden");
        let _tail = host.remove_attribute("data-tail");
        let _reason = host.set_attribute("data-anchor-fallback", "named-check-panel");
        let portal = self.plate_bubbles.get_bounding_client_rect();
        let scale_x = if self.plate_bubbles.client_width() > 0 {
            portal.width() / f64::from(self.plate_bubbles.client_width())
        } else {
            1.0
        };
        let scale_y = if self.plate_bubbles.client_height() > 0 {
            portal.height() / f64::from(self.plate_bubbles.client_height())
        } else {
            1.0
        };
        let mut avoid = Vec::new();
        for selector in BUBBLE_UI_OBSTACLES.into_iter().chain([
            ".tv-hud[data-party-hud=visible]",
            ".tv-lower",
            ".tv-dm",
        ]) {
            if let Ok(Some(node)) = self
                .root
                .parent_element()
                .unwrap_or_else(|| self.root.clone())
                .query_selector(selector)
                && let Ok(Some(style)) = window.get_computed_style(&node)
                && style.get_property_value("display").ok().as_deref() != Some("none")
                && (matches!(
                    node.get_attribute("data-line-state").as_deref(),
                    Some("opening" | "speaking" | "lingering")
                ) || (style.get_property_value("visibility").ok().as_deref() != Some("hidden")
                    && style.get_property_value("opacity").ok().as_deref() != Some("0")))
            {
                let bounds = node.get_bounding_client_rect();
                if bounds.width() > 0.0 && bounds.height() > 0.0 {
                    avoid.push(Rect::new(
                        bounds.left(),
                        bounds.top(),
                        bounds.right(),
                        bounds.bottom(),
                    ));
                }
            }
        }
        for (maximum_width, scroll) in [(600.0_f64, false), (960.0_f64, false), (960.0_f64, true)] {
            let overflow = if scroll {
                "max-height:25vh;overflow:auto;"
            } else {
                ""
            };
            let _measure = host.set_attribute(
                "style",
                &format!(
                    "left:0;top:0;right:auto;pointer-events:auto;width:{}px;{overflow}",
                    (width - 32.0).min(maximum_width) / scale_x
                ),
            );
            let size = host.get_bounding_client_rect();
            if let Some(rect) = plate_layout::place_panel(
                Rect::new(0.0, 0.0, width, height),
                (size.width(), size.height()),
                &avoid,
            ) {
                let _placed = host.set_attribute(
                    "style",
                    &format!(
                        "left:{}px;top:{}px;right:auto;pointer-events:auto;width:{}px;{overflow}",
                        (rect.left - portal.left()) / scale_x,
                        (rect.top - portal.top()) / scale_y,
                        rect.width() / scale_x
                    ),
                );
                return;
            }
        }
        let _hidden = host.set_attribute("hidden", "");
        let _reason = host.set_attribute("data-anchor-fallback", "check-panel-no-clear-space");
    }

    /// Hidden/zero/offscreen boxes are not screen positions and must never become fixed widths.
    fn retained_caption_bounds(&self, host: &Element) -> Option<Rect> {
        let window = web_sys::window()?;
        let screen = Rect::new(
            0.0,
            0.0,
            window.inner_width().ok()?.as_f64()?,
            window.inner_height().ok()?.as_f64()?,
        );
        let mut ancestor = Some(host.clone());
        while let Some(element) = ancestor {
            if element.has_attribute("hidden") {
                return None;
            }
            let style = window.get_computed_style(&element).ok()??;
            if style.get_property_value("display").ok()?.as_str() == "none"
                || style.get_property_value("visibility").ok()?.as_str() == "hidden"
                || style
                    .get_property_value("opacity")
                    .ok()?
                    .parse::<f64>()
                    .ok()
                    .is_some_and(|opacity| opacity <= 0.0)
            {
                return None;
            }
            ancestor = element.parent_element();
        }
        let bounds = host.get_bounding_client_rect();
        let rect = Rect::new(bounds.left(), bounds.top(), bounds.right(), bounds.bottom());
        (rect.width() >= 120.0
            && rect.height() > 0.0
            && screen.contains(&rect)
            && [rect.left, rect.top, rect.right, rect.bottom]
                .iter()
                .all(|value| value.is_finite()))
        .then_some(rect)
    }

    fn place_bubble_at_actor(&self, speaker: &Figure) {
        if let Some(id) = speaker.bubbles.get_attribute("data-anchor-line") {
            if speaker.bubbles.query_selector(&format!("[data-line-id=\"{id}\"]:is([data-line-state=opening],[data-line-state=speaking],[data-line-state=lingering])")).ok().flatten().is_some() {
                if self.retained_caption_bounds(&speaker.bubbles).is_none() {
                    let _hidden = speaker.bubbles.set_attribute("hidden", "");
                    let _cleared = speaker.bubbles.remove_attribute("style");
                }
                return;
            }
            let _removed = speaker.bubbles.remove_attribute("data-anchor-line");
        }
        let _shown = speaker.bubbles.remove_attribute("hidden");
        // PLAY-V09: over a scene plate the bubble stands beside the speaker's head in the picture.
        let geometry = self.plate.borrow().clone();
        if geometry.is_none() && self.plate_size.get().is_some() {
            let _hidden = speaker.bubbles.set_attribute("hidden", "");
            let _reason = speaker
                .bubbles
                .set_attribute("data-anchor-fallback", "missing-image-layout");
            return;
        }
        if let Some(geometry) = geometry {
            if let Some((picture_width, picture_height)) = self.plate_size.get() {
                self.place_bubble_on_plate(speaker, geometry, picture_width, picture_height);
            }
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
        let (left, top, right, bottom) = rectangle(&speaker.body);
        let (span, tall) = (right - left, bottom - top);
        let own_head = (
            left + span * 0.2,
            top,
            right - span * 0.2,
            top + tall * 0.36,
        );
        avoid.push(own_head);
        let mut fixed_avoid = vec![own_head];
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
            // A thinking marker or an earlier line is a live part of the stage too. Keep the
            // next bubble out of that rectangle; otherwise a reply can paint over the courier's
            // loader while the server is still resolving the reply.
            if other.bubbles.has_child_nodes() {
                let active = rectangle(&other.bubbles);
                if active.2 > active.0 && active.3 > active.1 {
                    avoid.push(active);
                }
            }
        }
        for selector in BUBBLE_FIXED_OBSTACLES {
            if let Ok(Some(panel)) = self.document.query_selector(selector) {
                fixed_avoid.push(rectangle(&panel));
            }
        }
        for selector in BUBBLE_UI_OBSTACLES {
            if let Ok(Some(panel)) = self.document.query_selector(selector) {
                avoid.push(rectangle(&panel));
            }
        }
        // The first pass is the normal placement path. Keep the title and rules note in this
        // same obstacle set as the cast heads; otherwise a bubble that fits beside a head can
        // still be painted underneath a fixed header and visibly collide with its labels.
        avoid.extend(fixed_avoid.iter().copied());
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
        for narrow in [false, true] {
            for side in sides {
                let _set = speaker.bubbles.set_attribute("data-side", side);
                flag(&speaker.bubbles, "data-narrow", narrow).ok();
                let placed = rectangle(&speaker.bubbles);
                let inside = placed.0 >= screen.0
                    && placed.2 <= screen.2
                    && placed.1 >= screen.1
                    && placed.3 <= screen.3;
                if inside && !fixed_avoid.iter().any(|head| overlaps(placed, *head)) {
                    return;
                }
            }
        }
        let _hidden = speaker.bubbles.set_attribute("hidden", "");
    }

    /// A retained caption must not point at the old position in a different photograph.
    /// Its text stays in the named lane until a successor can use the new image's geometry.
    pub(super) fn retain_plate_captions_in_named_lane(&self) -> Result<(), JsValue> {
        let Some(stage) = self.root.parent_element() else {
            return Ok(());
        };
        let current = stage.get_attribute("data-line-id");
        for figure in &self.figures {
            if let Some(line) = figure.bubbles.query_selector(".tv-bubble:is([data-line-state=opening],[data-line-state=speaking],[data-line-state=lingering])")?
                && let Some(id) = line.get_attribute("data-line-id") {
                figure.bubbles.set_attribute("data-anchor-line", &id)?;
                figure.bubbles.set_attribute("hidden", "")?;
                figure.bubbles.set_attribute("data-anchor-fallback", "bitmap-changed")?;
                if current.as_deref() == Some(id.as_str()) {
                    stage.set_attribute("data-unplaced-line", &id)?;
                }
            }
        }
        Ok(())
    }

    /// Same-bitmap measurements become available to successors without moving readable text.
    pub(super) fn latch_plate_geometry(
        &self,
        geometry: Option<PlateGeometry>,
        picture_size: Option<(f64, f64)>,
    ) -> Result<(), JsValue> {
        if *self.plate.borrow() == geometry && self.plate_size.get() == picture_size {
            return Ok(());
        }
        for figure in &self.figures {
            if let Some(line) = figure.bubbles.query_selector(".tv-bubble[data-line-id]:is([data-line-state=opening],[data-line-state=speaking],[data-line-state=lingering])")?
                && let Some(id) = line.get_attribute("data-line-id") {
                figure.bubbles.set_attribute("data-anchor-line", &id)?;
            }
        }
        self.set_plate(geometry, picture_size)
    }

    /// PLAY-V09: tells the stage that a scene plate with this layout covers the cut-outs (or, with
    /// `None`, that none does). The bubbles then stand in the plate's own layer and are placed
    /// from the layout; without a plate they return to their figures.
    pub(super) fn set_plate(
        &self,
        geometry: Option<PlateGeometry>,
        picture_size: Option<(f64, f64)>,
    ) -> Result<(), JsValue> {
        let before = self.plate.replace(geometry.clone());
        let before_size = self.plate_size.replace(picture_size);
        if before == geometry && before_size == picture_size {
            return Ok(());
        }
        for figure in &self.figures {
            let host_is_plate = figure
                .bubbles
                .parent_element()
                .is_some_and(|parent| parent.is_same_node(Some(self.plate_bubbles.as_ref())));
            if picture_size.is_some() && !host_is_plate {
                // Moving a bubble in the page restarts its entrance animation: the line being
                // spoken must not pop in again as the plate takes over, so its bubbles skip it.
                let mut bubble = figure.bubbles.first_element_child();
                while let Some(open) = bubble {
                    open.set_attribute("data-instant", "")?;
                    bubble = open.next_element_sibling();
                }
                let retained = figure
                    .bubbles
                    .has_attribute("data-anchor-line")
                    .then(|| self.retained_caption_bounds(&figure.bubbles))
                    .flatten();
                self.plate_bubbles.append_child(&figure.bubbles)?;
                if let Some(bounds) = retained {
                    let portal = self.plate_bubbles.get_bounding_client_rect();
                    let sx = if portal.width() > 0.0 && self.plate_bubbles.client_width() > 0 {
                        portal.width() / f64::from(self.plate_bubbles.client_width())
                    } else {
                        1.0
                    };
                    let sy = if portal.height() > 0.0 && self.plate_bubbles.client_height() > 0 {
                        portal.height() / f64::from(self.plate_bubbles.client_height())
                    } else {
                        1.0
                    };
                    figure.bubbles.set_attribute(
                        "style",
                        &format!(
                            "left:{}px;top:{}px;width:{}px;right:auto;bottom:auto;transform:none",
                            (bounds.left - portal.left()) / sx,
                            (bounds.top - portal.top()) / sy,
                            bounds.width() / sx
                        ),
                    )?;
                } else if figure.bubbles.has_attribute("data-anchor-line") {
                    figure.bubbles.remove_attribute("style")?;
                    figure.bubbles.set_attribute("hidden", "")?;
                    figure
                        .bubbles
                        .set_attribute("data-anchor-fallback", "unmeasurable-retained-caption")?;
                }
            } else if picture_size.is_none() && host_is_plate {
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
    fn place_bubble_on_plate(
        &self,
        bubble_owner: &Figure,
        geometry: PlateGeometry,
        picture_width: f64,
        picture_height: f64,
    ) {
        let _shown = bubble_owner.bubbles.remove_attribute("hidden");
        if let Some(stage) = self.root.parent_element()
            && let Some(key) = stage.get_attribute("data-plate-key")
        {
            let _key = bubble_owner
                .bubbles
                .set_attribute("data-anchor-image-key", &key);
        }
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
        let portal = self.plate_bubbles.get_bounding_client_rect();
        let scale = |screen: f64, css: i32| {
            if screen > 0.0 && css > 0 {
                screen / f64::from(css)
            } else {
                1.0
            }
        };
        let scale_x = scale(portal.width(), self.plate_bubbles.client_width());
        let scale_y = scale(portal.height(), self.plate_bubbles.client_height());
        let active_image = self.root.parent_element().and_then(|stage| {
            let plate = stage.query_selector(".tv-plate").ok().flatten()?;
            let active = plate.get_attribute("data-active")?.parse::<usize>().ok()?;
            plate
                .query_selector(&format!(
                    ".tv-plate-frame[data-i=\"{active}\"] .tv-plate-img"
                ))
                .ok()
                .flatten()
        });
        let image_bounds = active_image
            .map(|image| image.get_bounding_client_rect())
            .filter(|bounds| bounds.width() > 0.0 && bounds.height() > 0.0);
        let picture_screen = image_bounds.map_or(screen, |bounds| {
            Rect::new(bounds.left(), bounds.top(), bounds.right(), bounds.bottom())
        });
        let frame = PlateFrame {
            screen: picture_screen,
            picture_width,
            picture_height,
        };
        let speaker_head = frame.head_box(
            geometry.head_x,
            geometry.head_y,
            geometry.head_width,
            geometry.head_height,
        );
        let mut fallback_avoid = vec![speaker_head];
        let slot = bubble_owner
            .root
            .get_attribute("data-slot")
            .unwrap_or_default();
        // A missing measured identity uses the named lane; a cut-out or HUD position cannot
        // identify the person in this exact photograph.
        let hero_index = slot
            .strip_prefix("hero-")
            .and_then(|number| number.parse::<usize>().ok())
            .and_then(|number| number.checked_sub(1));
        let (head, side, mut avoid) = match hero_index {
            Some(index) => match geometry.heroes.iter().find(|hero| hero.index == index) {
                Some(hero) => (
                    frame.head_box(hero.head_x, hero.head_y, hero.head_width, hero.head_height),
                    hero.bubble_side,
                    vec![speaker_head],
                ),
                None => {
                    let _hidden = bubble_owner.bubbles.set_attribute("hidden", "");
                    let _reason = bubble_owner
                        .bubbles
                        .set_attribute("data-anchor-fallback", "missing-hero-head");
                    return;
                }
            },
            None => {
                let id = bubble_owner
                    .root
                    .get_attribute("data-actor-id")
                    .unwrap_or_default();
                let Some(actor) = geometry
                    .actors
                    .iter()
                    .find(|actor| actor.actor_id == id && !id.is_empty())
                else {
                    let _hidden = bubble_owner.bubbles.set_attribute("hidden", "");
                    let _reason = bubble_owner
                        .bubbles
                        .set_attribute("data-anchor-fallback", "missing-actor-head");
                    return;
                };
                let _reason = bubble_owner
                    .bubbles
                    .remove_attribute("data-anchor-fallback");
                (
                    frame.head_box(
                        actor.head_x,
                        actor.head_y,
                        actor.head_width,
                        actor.head_height,
                    ),
                    actor.bubble_side,
                    Vec::new(),
                )
            }
        };
        for hero in &geometry.heroes {
            let box_ = frame.head_box(hero.head_x, hero.head_y, hero.head_width, hero.head_height);
            if Some(hero.index) != hero_index {
                avoid.push(box_);
            }
            fallback_avoid.push(box_);
        }
        let owner_id = bubble_owner
            .root
            .get_attribute("data-actor-id")
            .unwrap_or_default();
        for actor in &geometry.actors {
            let box_ = frame.head_box(
                actor.head_x,
                actor.head_y,
                actor.head_width,
                actor.head_height,
            );
            if actor.actor_id != owner_id {
                avoid.push(box_);
            }
            fallback_avoid.push(box_);
        }
        let mut interface = Vec::new();
        let mut keep_clear = |element: &Element| {
            let (left, top, right, bottom) = rectangle(element);
            if right > left && bottom > top {
                interface.push(Rect::new(left, top, right, bottom));
            }
        };
        for card in &self.hud {
            if card.root.get_attribute("data-present").as_deref() == Some("true")
                && card
                    .root
                    .closest(".tv-hud[data-party-hud=visible]")
                    .ok()
                    .flatten()
                    .is_some()
            {
                let bounds = card.root.get_bounding_client_rect();
                fallback_avoid.push(Rect::new(
                    bounds.left(),
                    bounds.top(),
                    bounds.right(),
                    bounds.bottom(),
                ));
                keep_clear(&card.root);
            }
        }
        for selector in BUBBLE_UI_OBSTACLES {
            if let Ok(Some(element)) = self.document.query_selector(selector) {
                keep_clear(&element);
            }
        }
        for selector in BUBBLE_FIXED_OBSTACLES {
            if let Ok(Some(element)) = self.document.query_selector(selector) {
                let (left, top, right, bottom) = rectangle(&element);
                if right > left && bottom > top {
                    fallback_avoid.push(Rect::new(left, top, right, bottom));
                }
            }
        }
        // The plate has its own bubble layer, so the regular figure-head obstacles do not cover
        // another figure's live speech or thinking bubble. Include those active bubbles before
        // asking the plate layout to select a side.
        for figure in &self.figures {
            if figure.root.is_same_node(Some(bubble_owner.root.as_ref()))
                || !Self::present(figure)
                || !figure.bubbles.has_child_nodes()
            {
                continue;
            }
            let active = rectangle(&figure.bubbles);
            if active.2 > active.0 && active.3 > active.1 {
                interface.push(Rect::new(active.0, active.1, active.2, active.3));
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
                f64::from(container.client_width()) * scale_x,
                f64::from(container.client_height()) * scale_y,
            );
            placement =
                plate_layout::place_bubble_near_head(screen, head, side, size, tail_offset, &avoid);
            if placement.is_some() {
                break;
            }
        }
        if placement.is_none() {
            // No clear place at all (a huge bubble): keep off the head and the screen edge only.
            let size = (
                f64::from(container.client_width()) * scale_x,
                f64::from(container.client_height()) * scale_y,
            );
            placement = plate_layout::place_bubble_near_head(
                screen,
                head,
                side,
                size,
                tail_offset,
                &fallback_avoid,
            );
        }
        let Some(placed) = placement else {
            let _hidden = bubble_owner.bubbles.set_attribute("hidden", "");
            return;
        };
        let _set = container.set_attribute("data-side", placed.side.name());
        let (edge, tail_x, tail_y) = placed.tail(head);
        let _set = container.set_attribute("data-tail", edge);
        let _set = container.set_attribute(
            "style",
            &format!(
                "left:{:.1}px;top:{:.1}px;right:auto;--tail-x:{:.1}px;--tail-y:{:.1}px",
                (placed.rect.left - portal.left()) / scale_x,
                (placed.rect.top - portal.top()) / scale_y,
                tail_x / scale_x,
                tail_y / scale_y
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
        let other_host = if combat { &figure.bubbles } else { &card.root };
        let selector = format!("[data-line-id=\"{id}\"]");
        let existing = host
            .query_selector(&selector)?
            .or(other_host.query_selector(&selector)?);
        let created = existing.is_none();
        let live = matches!(state, LineState::Composing | LineState::Submitted);
        if live {
            figure.root.set_attribute("data-intent", state.name())?;
        } else {
            figure.root.remove_attribute("data-intent")?;
        }
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
        let component_phase = match state {
            LineState::Composing | LineState::Submitted => {
                if created {
                    ComponentPhase::Opened
                } else {
                    ComponentPhase::Readable
                }
            }
            LineState::Cancelled | LineState::Closing => ComponentPhase::Closing,
            LineState::Done => ComponentPhase::Hidden,
            LineState::Queued => ComponentPhase::Queued,
            _ => ComponentPhase::Opened,
        };
        let scene = self.scene.borrow().clone();
        loader.set_attribute("data-component-id", &id.to_string())?;
        loader.set_attribute("data-component-scene", &scene)?;
        loader.set_attribute("data-component-type", "intent_loader")?;
        let reason = if state == LineState::Done {
            "ended"
        } else if matches!(state, LineState::Closing | LineState::Cancelled) {
            "successor"
        } else {
            "ready"
        };
        if created {
            super::presentation_timing::component(
                None,
                PresentationComponent::IntentLoader,
                id,
                &scene,
                ComponentPhase::Requested,
                "ready",
                0,
            );
        }
        if state == LineState::Done {
            loader.remove();
        }
        super::presentation_timing::component(
            Some(&loader),
            PresentationComponent::IntentLoader,
            id,
            &scene,
            component_phase,
            reason,
            0,
        );
        if created && live {
            super::presentation_timing::schedule_component_transition(
                loader,
                PresentationComponent::IntentLoader,
                id,
                scene,
                ComponentPhase::Readable,
                "ready",
                0,
                200,
            );
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
        if matches!(
            state,
            LineState::Closing | LineState::Done | LineState::Cancelled
        ) {
            self.set_unplaced_line_strip(id, state, false)?;
        }
        let component_type = PresentationComponent::SpeechBubble;
        let phase = line_component_phase(state);
        let reason = line_component_reason(state);
        let read_ms = crate::stage_machines::reading_time_ms(bubble.text);
        let anchor = if bubble.kind == BubbleKind::Typed {
            let heroes = self
                .figures
                .iter()
                .take(HEROES)
                .map(|figure| Self::present(figure).then(|| figure.name_text.borrow().clone()))
                .collect::<Vec<_>>();
            typed_line_anchor(bubble.speaker, &heroes)
        } else {
            Some(position)
        };
        let Some(figure) = anchor.and_then(|position| self.figures.get(position)) else {
            if bubble.kind == BubbleKind::Typed {
                let opened = self.set_typed_line_strip(id, state)?;
                super::presentation_timing::line(
                    id,
                    state,
                    None,
                    bubble.text.chars().count() as u32,
                    read_ms,
                    &self.scene.borrow(),
                );
                let lower = self
                    .root
                    .parent_element()
                    .and_then(|stage| stage.query_selector(".tv-lower").ok().flatten());
                let scene = self.scene.borrow().clone();
                if let Some(lower) = lower.as_ref() {
                    lower.set_attribute("data-component-id", &id.to_string())?;
                    lower.set_attribute("data-component-scene", &scene)?;
                    lower.set_attribute("data-component-type", "speech_bubble")?;
                }
                if opened {
                    super::presentation_timing::component(
                        None,
                        PresentationComponent::SpeechBubble,
                        id,
                        &scene,
                        ComponentPhase::Requested,
                        "ready",
                        read_ms,
                    );
                }
                super::presentation_timing::component(
                    lower.as_ref(),
                    PresentationComponent::SpeechBubble,
                    id,
                    &scene,
                    phase,
                    reason,
                    read_ms,
                );
                if state == LineState::Opening
                    && opened
                    && let Some(lower) = lower
                {
                    super::presentation_timing::schedule_component_transition(
                        lower,
                        PresentationComponent::SpeechBubble,
                        id,
                        scene,
                        ComponentPhase::Readable,
                        "ready",
                        read_ms,
                        500,
                    );
                } else if state == LineState::Closing
                    && let Some(lower) = lower
                {
                    super::presentation_timing::schedule_component_transition(
                        lower,
                        PresentationComponent::SpeechBubble,
                        id,
                        scene,
                        ComponentPhase::Hidden,
                        "successor",
                        read_ms,
                        500,
                    );
                }
                return Ok(opened);
            }
            return Ok(false);
        };
        if bubble.kind == BubbleKind::Typed
            && let Some(stage) = self.root.parent_element()
        {
            stage.remove_attribute("data-typed-strip")?;
        }
        let existing = figure
            .bubbles
            .query_selector(&format!("[data-line-id=\"{id}\"]"))?;
        let (element, opened) = match (existing, state) {
            (Some(element), _) => (element, false),
            (None, LineState::Opening | LineState::Speaking | LineState::Lingering) => {
                (self.open_managed(figure, id, state, bubble)?, true)
            }
            (None, LineState::Closing | LineState::Done | LineState::Cancelled) => {
                let scene = self.scene.borrow().clone();
                super::presentation_timing::line(
                    id,
                    state,
                    anchor,
                    bubble.text.chars().count() as u32,
                    read_ms,
                    &scene,
                );
                super::presentation_timing::component(
                    None,
                    component_type,
                    id,
                    &scene,
                    phase,
                    reason,
                    read_ms,
                );
                return Ok(false);
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
        if matches!(
            state,
            LineState::Opening | LineState::Speaking | LineState::Lingering
        ) {
            self.place_bubble(figure);
        }
        self.set_unplaced_line_strip(id, state, figure.bubbles.has_attribute("hidden"))?;
        super::presentation_timing::line(
            id,
            state,
            anchor,
            bubble.text.chars().count() as u32,
            read_ms,
            &self.scene.borrow(),
        );
        let scene = self.scene.borrow().clone();
        element.set_attribute("data-component-id", &id.to_string())?;
        element.set_attribute("data-component-scene", &scene)?;
        element.set_attribute("data-component-type", "speech_bubble")?;
        if opened {
            super::presentation_timing::component(
                None,
                component_type,
                id,
                &scene,
                ComponentPhase::Requested,
                "ready",
                read_ms,
            );
        }
        super::presentation_timing::component(
            Some(&element),
            component_type,
            id,
            &scene,
            phase,
            reason,
            read_ms,
        );
        if state == LineState::Opening && opened {
            super::presentation_timing::schedule_component_transition(
                element,
                component_type,
                id,
                scene,
                ComponentPhase::Readable,
                "ready",
                read_ms,
                200,
            );
        } else if state == LineState::Closing {
            super::presentation_timing::schedule_component_transition(
                element,
                component_type,
                id,
                scene,
                ComponentPhase::Hidden,
                reason,
                read_ms,
                200,
            );
        }
        Ok(opened)
    }

    /// A line that cannot fit safely beside its head remains readable in the named lane.
    fn set_unplaced_line_strip(
        &self,
        id: u64,
        state: LineState,
        unplaced: bool,
    ) -> Result<(), JsValue> {
        let Some(stage) = self.root.parent_element() else {
            return Ok(());
        };
        let identity = id.to_string();
        let owns_strip = stage.get_attribute("data-unplaced-line").as_deref() == Some(&identity);
        if matches!(
            state,
            LineState::Opening | LineState::Speaking | LineState::Lingering
        ) {
            if unplaced {
                stage.set_attribute("data-unplaced-line", &identity)?;
            } else {
                // The newly placed line replaces any predecessor's fallback lane.
                stage.remove_attribute("data-unplaced-line")?;
            }
        } else if owns_strip {
            stage.remove_attribute("data-unplaced-line")?;
        }
        Ok(())
    }

    fn set_typed_line_strip(&self, id: u64, state: LineState) -> Result<bool, JsValue> {
        let Some(stage) = self.root.parent_element() else {
            return Ok(false);
        };
        let line_id = id.to_string();
        let already_shown = stage.get_attribute("data-typed-strip").as_deref() == Some(&line_id);
        if state == LineState::Done {
            if already_shown {
                stage.remove_attribute("data-typed-strip")?;
                stage.remove_attribute("data-typed-strip-closing")?;
            }
        } else if state != LineState::Closing {
            stage.set_attribute("data-typed-strip", &line_id)?;
            stage.remove_attribute("data-typed-strip-closing")?;
        }
        Ok(!already_shown && state != LineState::Closing && state != LineState::Done)
    }

    fn open_managed(
        &self,
        figure: &Figure,
        id: u64,
        state: LineState,
        bubble: &LineBubble<'_>,
    ) -> Result<Element, JsValue> {
        let replaced_typed_line = if bubble.kind == BubbleKind::Speech {
            let mut replaced = false;
            for speaker in &self.figures {
                if let Some(typed) = speaker
                    .bubbles
                    .query_selector("[data-kind=typed][data-managed]")?
                {
                    typed.set_attribute("data-line-state", LineState::Closing.name())?;
                    replaced = true;
                }
            }
            if let Some(stage) = self.root.parent_element()
                && should_close_typed_strip(bubble.kind, stage.has_attribute("data-typed-strip"))
            {
                stage.set_attribute("data-typed-strip-closing", "")?;
                replaced = true;
            }
            replaced
        } else {
            false
        };
        let position = self
            .figures
            .iter()
            .position(|candidate| candidate.root.is_same_node(Some(&figure.root)))
            .unwrap_or_default();
        self.clear_figure_components(figure, position, "successor")?;
        let element = node(&self.document, &figure.bubbles, "div", "tv-bubble", "")?;
        element.set_attribute("data-kind", bubble.kind.name())?;
        element.set_attribute("data-line-id", &id.to_string())?;
        element.set_attribute("data-managed", "")?;
        let scene = self.scene.borrow().clone();
        element.set_attribute("data-component-id", &id.to_string())?;
        element.set_attribute("data-component-scene", &scene)?;
        element.set_attribute("data-component-type", "speech_bubble")?;
        if replaced_typed_line {
            element.set_attribute("style", "--appear:180ms")?;
        }
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
        if let Some(stage) = self.root.parent_element() {
            stage.remove_attribute("data-unplaced-line")?;
        }
        for (position, figure) in self.figures.iter().enumerate() {
            self.clear_figure_components(figure, position, "scene-change")?;
        }
        Ok(())
    }

    /// Shows the small thinking indicator at the figure in `slot` (a speaker writing a reply),
    /// or at none. Showing it where it already is changes nothing, so its animation is not
    /// restarted by every view.
    pub(super) fn set_thinking(&self, slot: Option<ActorSlot>) -> Result<(), JsValue> {
        let target = slot.and_then(|slot| self.figure(slot));
        for (position, figure) in self.figures.iter().enumerate() {
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
                let id = position as u64 + 1;
                let scene = self.scene.borrow().clone();
                dots.set_attribute("data-component-id", &id.to_string())?;
                dots.set_attribute("data-component-scene", &scene)?;
                dots.set_attribute("data-component-type", "thinking_loader")?;
                super::presentation_timing::component(
                    None,
                    PresentationComponent::ThinkingLoader,
                    id,
                    &scene,
                    ComponentPhase::Requested,
                    "ready",
                    0,
                );
                super::presentation_timing::component(
                    Some(&dots),
                    PresentationComponent::ThinkingLoader,
                    id,
                    &scene,
                    ComponentPhase::Opened,
                    "ready",
                    0,
                );
                super::presentation_timing::schedule_component_transition(
                    dots,
                    PresentationComponent::ThinkingLoader,
                    id,
                    scene,
                    ComponentPhase::Readable,
                    "ready",
                    0,
                    200,
                );
            } else if !wanted && shown {
                figure.root.remove_attribute("data-thinking")?;
                if let Some(dots) = figure.bubbles.query_selector(".tv-thinking")? {
                    dots.remove();
                    let id = position as u64 + 1;
                    let scene = self.scene.borrow().clone();
                    super::presentation_timing::component(
                        Some(&dots),
                        PresentationComponent::ThinkingLoader,
                        id,
                        &scene,
                        ComponentPhase::Hidden,
                        "successor",
                        0,
                    );
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

fn healing_action_text(mode: StageMode, amount: u32) -> String {
    let action = match mode {
        StageMode::Combat => "catches a second wind",
        StageMode::Aftermath | StageMode::Scene => "rests and recovers",
    };
    format!("{action} · +{amount} HP")
}

#[cfg(test)]
mod tests {
    use super::{
        BUBBLE_FIXED_OBSTACLES, CHECK_FADE_MS, CHECK_FINDING_REVEAL_MS, CHECK_ROLL_MS,
        art_candidates, check_result_phase_ms, line_component_phase, typed_line_anchor,
    };
    use df_observe::presentation::ComponentPhase;

    #[test]
    fn check_without_image_heads_has_a_measured_named_panel_with_roll_and_finding() {
        use super::{CheckDisplay, Figures};
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        let parent = document.create_element("section").unwrap();
        parent.set_attribute("class", "tv-stage").unwrap();
        parent.set_attribute("data-tv-mode", "scene").unwrap();
        parent.set_attribute("data-shot", "plate").unwrap();
        parent
            .set_attribute("style", "position:relative;width:100vw;height:100vh")
            .unwrap();
        document.body().unwrap().append_child(&parent).unwrap();
        let style = document.create_element("style").unwrap();
        style.set_text_content(Some(&format!(
            "{}\n{}",
            include_str!("live_stage.css"),
            include_str!("live_plate.css")
        )));
        parent.append_child(&style).unwrap();
        let figures = Figures::mount(&document, &parent).unwrap();
        let hero = &figures.figures[0];
        hero.root.set_attribute("data-present", "true").unwrap();
        *hero.name_text.borrow_mut() = "Brakka".into();
        figures.set_plate(None, Some((1920.0, 1080.0))).unwrap();
        figures
            .set_check(Some(&CheckDisplay {
                sequence: 5,
                hero_name: "Brakka".into(),
                intention: "Search the sealed room".into(),
                ability_skill: "Investigation".into(),
                faces: vec![(8, true)],
                roll_mode: 0,
                modifier: 0,
                total: 8,
                succeeded: false,
                finding: "The seal is intact; the crest is visible.".into(),
            }))
            .unwrap();
        assert!(!hero.bubbles.has_attribute("hidden"));
        assert_eq!(
            hero.bubbles
                .get_attribute("data-anchor-fallback")
                .as_deref(),
            Some("named-check-panel")
        );
        let panel = hero
            .bubbles
            .query_selector(".tv-check-result")
            .unwrap()
            .unwrap();
        assert_eq!(
            panel
                .query_selector(".tv-check-hero")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Brakka")
        );
        assert_eq!(
            panel
                .query_selector(".tv-check-intention")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Search the sealed room")
        );
        assert_eq!(
            panel
                .query_selector(".tv-check-total")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("8")
        );
        assert!(
            panel
                .query_selector(".tv-check-finding")
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap()
                .contains("seal is intact")
        );
        let bounds = hero.bubbles.get_bounding_client_rect();
        assert!(bounds.width() > 0.0 && bounds.height() > 0.0);
        assert!(bounds.left() >= 0.0 && bounds.top() >= 0.0);
        assert!(bounds.right() <= window.inner_width().unwrap().as_f64().unwrap());
        assert!(bounds.bottom() <= window.inner_height().unwrap().as_f64().unwrap());
        figures.dispose();
        parent.remove();
    }

    #[test]
    fn managed_checks_reveal_and_finish_only_the_matching_queued_payload() {
        use super::{CheckDisplay, Figures};
        let document = web_sys::window().unwrap().document().unwrap();
        let parent = document.create_element("section").unwrap();
        document.body().unwrap().append_child(&parent).unwrap();
        let figures = Figures::mount(&document, &parent).unwrap();
        let hero = &figures.figures[0];
        hero.root.set_attribute("data-present", "true").unwrap();
        *hero.name_text.borrow_mut() = "Brakka".into();
        let mut first = CheckDisplay {
            sequence: 5,
            hero_name: "Brakka".into(),
            intention: "Inspect the seal".into(),
            ability_skill: "Investigation".into(),
            faces: vec![(8, true)],
            roll_mode: 0,
            modifier: 0,
            total: 8,
            succeeded: false,
            finding: "The seal remains intact.".into(),
        };
        figures.begin_check(&first, false).unwrap();
        let old = hero
            .bubbles
            .query_selector("[data-check-sequence=\"5\"]")
            .unwrap()
            .unwrap();
        assert_eq!(
            old.get_attribute("data-check-phase").as_deref(),
            Some("roll")
        );
        assert!(old.has_attribute("data-check-managed"));
        first.sequence = 6;
        figures.begin_check(&first, false).unwrap();
        let next = hero
            .bubbles
            .query_selector("[data-check-sequence=\"6\"]")
            .unwrap()
            .unwrap();
        figures.reveal_check(5).unwrap();
        assert_eq!(
            old.get_attribute("data-check-phase").as_deref(),
            Some("result")
        );
        assert_eq!(
            next.get_attribute("data-check-phase").as_deref(),
            Some("roll")
        );
        figures.finish_check(5).unwrap();
        assert!(!old.is_connected());
        assert!(next.is_connected());
        figures.reveal_check(6).unwrap();
        figures.finish_check(5).unwrap();
        assert!(
            next.is_connected(),
            "stale predecessor finish cannot retire successor card"
        );
        figures.finish_check(6).unwrap();
        first.sequence = 7;
        figures.begin_check(&first, true).unwrap();
        assert_eq!(
            hero.bubbles
                .query_selector("[data-check-sequence=\"7\"]")
                .unwrap()
                .unwrap()
                .get_attribute("data-check-phase")
                .as_deref(),
            Some("result")
        );
        figures.dispose();
        parent.remove();
    }

    #[test]
    fn retired_check_does_not_replay_or_replace_the_next_line_but_new_roll_does() {
        use super::{BubbleKind, CheckDisplay, Figures, LineBubble};
        use crate::stage_machines::LineState;
        let document = web_sys::window().unwrap().document().unwrap();
        let parent = document.create_element("section").unwrap();
        document.body().unwrap().append_child(&parent).unwrap();
        let figures = Figures::mount(&document, &parent).unwrap();
        let hero = &figures.figures[0];
        hero.root.set_attribute("data-present", "true").unwrap();
        *hero.name_text.borrow_mut() = "Brakka".to_owned();
        let mut check = CheckDisplay {
            sequence: 5,
            hero_name: "Brakka".to_owned(),
            intention: "Look for a courier".to_owned(),
            ability_skill: "Intelligence (Investigation)".to_owned(),
            faces: vec![(18, true)],
            roll_mode: 0,
            modifier: 0,
            total: 18,
            succeeded: true,
            finding: "Courier is here.".to_owned(),
        };
        figures.set_check(Some(&check)).unwrap();
        hero.bubbles
            .query_selector(".tv-check-result")
            .unwrap()
            .unwrap()
            .remove();
        let line = LineBubble {
            kind: BubbleKind::Typed,
            speaker: "Brakka",
            text: "Tell me about the seal.",
            voiced: false,
        };
        figures.set_line(0, 3, LineState::Opening, &line).unwrap();
        figures.set_check(Some(&check)).unwrap();
        assert!(
            hero.bubbles
                .query_selector(".tv-check-result")
                .unwrap()
                .is_none()
        );
        assert!(
            hero.bubbles
                .query_selector("[data-line-id='3']")
                .unwrap()
                .is_some()
        );
        // An identical outcome at a later authoritative Roll sequence is a new event.
        check.sequence = 8;
        figures.set_check(Some(&check)).unwrap();
        assert!(
            hero.bubbles
                .query_selector(".tv-check-result")
                .unwrap()
                .is_some()
        );
        check.sequence = 9;
        figures.set_check(Some(&check)).unwrap();
        assert_eq!(
            hero.bubbles
                .query_selector_all(".tv-check-result")
                .unwrap()
                .length(),
            2
        );
        figures.dispose();
        parent.remove();
    }

    #[test]
    fn unplaceable_plate_line_uses_named_lane_and_next_fitting_line_restores_host() {
        use super::{BubbleKind, Figures, LineBubble};
        use crate::{plate_layout::Side, presentation::PlateGeometry, stage_machines::LineState};
        let document = web_sys::window().unwrap().document().unwrap();
        let parent = document.create_element("section").unwrap();
        parent.set_attribute("data-shot", "plate").unwrap();
        document.body().unwrap().append_child(&parent).unwrap();
        let figures = Figures::mount(&document, &parent).unwrap();
        let hero = &figures.figures[0];
        hero.root.set_attribute("data-present", "true").unwrap();
        *hero.name_text.borrow_mut() = "Brakka".to_owned();
        figures
            .set_plate(
                Some(PlateGeometry {
                    actors: Vec::new(),
                    bubble_side: Side::Right,
                    head_x: 500,
                    head_y: 200,
                    head_width: 100,
                    head_height: 100,
                    heroes: vec![],
                }),
                Some((1920.0, 1080.0)),
            )
            .unwrap();
        hero.bubbles
            .set_attribute("style", "width:10000px;height:10000px")
            .unwrap();
        let line = LineBubble {
            kind: BubbleKind::Typed,
            speaker: "Brakka",
            text: "A long question must remain readable.",
            voiced: false,
        };
        figures.set_line(0, 3, LineState::Opening, &line).unwrap();
        assert!(hero.bubbles.has_attribute("hidden"));
        assert_eq!(
            parent.get_attribute("data-unplaced-line").as_deref(),
            Some("3")
        );
        figures.set_line(0, 3, LineState::Lingering, &line).unwrap();
        assert_eq!(
            parent.get_attribute("data-unplaced-line").as_deref(),
            Some("3")
        );
        figures.set_line(0, 3, LineState::Done, &line).unwrap();
        assert!(!parent.has_attribute("data-unplaced-line"));
        hero.bubbles
            .set_attribute("style", "width:100px;height:40px")
            .unwrap();
        figures.set_line(0, 4, LineState::Opening, &line).unwrap();
        assert!(!hero.bubbles.has_attribute("hidden"));
        assert!(!parent.has_attribute("data-unplaced-line"));
        figures.dispose();
        parent.remove();
    }

    #[test]
    fn line_completion_waits_for_observed_dom_closure() {
        use crate::stage_machines::LineState;

        assert_eq!(
            line_component_phase(LineState::Composing),
            ComponentPhase::Requested
        );
        assert_eq!(
            line_component_phase(LineState::Queued),
            ComponentPhase::Queued
        );
        assert_eq!(
            line_component_phase(LineState::Opening),
            ComponentPhase::Opened
        );
        assert_eq!(
            line_component_phase(LineState::Speaking),
            ComponentPhase::Readable
        );
        assert_eq!(
            line_component_phase(LineState::Lingering),
            ComponentPhase::Readable
        );
        assert_eq!(
            line_component_phase(LineState::Closing),
            ComponentPhase::Closing
        );
        assert_eq!(
            line_component_phase(LineState::Done),
            ComponentPhase::Closing
        );
    }

    #[test]
    fn photographic_sprite_candidates_do_not_reintroduce_painted_portraits() {
        let candidates = art_candidates(
            "/assets/ui/generated/sprite-courier",
            "/assets/ui/generated/portrait-courier",
            true,
        );
        assert!(!candidates.is_empty());
        assert!(candidates.iter().all(|(_, path)| path.contains("photo")));
    }

    #[test]
    fn party_hud_tracks_authoritative_scene_combat_scene_boundaries() {
        use super::{Figures, SceneFamily, StageMode};
        let document = web_sys::window().unwrap().document().unwrap();
        let parent = document.create_element("section").unwrap();
        document.body().unwrap().append_child(&parent).unwrap();
        let figures = Figures::mount(&document, &parent).unwrap();
        let hud = figures.root.query_selector(".tv-hud").unwrap().unwrap();

        figures
            .update(&[], "scene", "inn", SceneFamily::Interior, StageMode::Scene)
            .unwrap();
        assert_eq!(
            hud.get_attribute("data-party-hud").as_deref(),
            Some("hidden")
        );
        figures
            .update(
                &[],
                "combat",
                "inn",
                SceneFamily::Interior,
                StageMode::Combat,
            )
            .unwrap();
        assert_eq!(
            hud.get_attribute("data-party-hud").as_deref(),
            Some("visible")
        );
        figures
            .update(&[], "scene", "inn", SceneFamily::Interior, StageMode::Scene)
            .unwrap();
        assert_eq!(
            hud.get_attribute("data-party-hud").as_deref(),
            Some("hidden")
        );

        figures.dispose();
        parent.remove();
    }

    #[test]
    fn party_hud_same_phase_repairs_dom_without_restarting_transition() {
        use super::{Figures, SceneFamily, StageMode};
        let document = web_sys::window().unwrap().document().unwrap();
        let parent = document.create_element("section").unwrap();
        document.body().unwrap().append_child(&parent).unwrap();
        let figures = Figures::mount(&document, &parent).unwrap();
        figures
            .update(
                &[],
                "combat",
                "inn",
                SceneFamily::Interior,
                StageMode::Combat,
            )
            .unwrap();
        let transition_id = figures.party_hud_transition_id.get();
        let hud = figures.root.query_selector(".tv-hud").unwrap().unwrap();
        hud.set_attribute("data-party-hud", "stale").unwrap();
        figures
            .update(
                &[],
                "combat",
                "inn",
                SceneFamily::Interior,
                StageMode::Combat,
            )
            .unwrap();
        assert_eq!(
            hud.get_attribute("data-party-hud").as_deref(),
            Some("visible")
        );
        assert_eq!(figures.party_hud_transition_id.get(), transition_id);

        figures.dispose();
        parent.remove();
    }

    #[test]
    fn party_hud_retargets_with_fresh_identity_and_reconnects_without_flash() {
        use super::{Figures, SceneFamily, StageMode};
        let document = web_sys::window().unwrap().document().unwrap();
        let parent = document.create_element("section").unwrap();
        document.body().unwrap().append_child(&parent).unwrap();
        let figures = Figures::mount(&document, &parent).unwrap();
        figures
            .update(
                &[],
                "combat",
                "inn",
                SceneFamily::Interior,
                StageMode::Combat,
            )
            .unwrap();
        let first_id = figures.party_hud_transition_id.get();
        figures
            .update(&[], "scene", "inn", SceneFamily::Interior, StageMode::Scene)
            .unwrap();
        figures
            .update(
                &[],
                "combat",
                "inn",
                SceneFamily::Interior,
                StageMode::Combat,
            )
            .unwrap();
        assert!(figures.party_hud_transition_id.get() > first_id);
        assert_eq!(
            figures
                .root
                .query_selector(".tv-hud")
                .unwrap()
                .unwrap()
                .get_attribute("data-party-hud")
                .as_deref(),
            Some("visible")
        );
        figures.dispose();
        parent.remove();

        let reconnect_parent = document.create_element("section").unwrap();
        document
            .body()
            .unwrap()
            .append_child(&reconnect_parent)
            .unwrap();
        let reconnect = Figures::mount(&document, &reconnect_parent).unwrap();
        reconnect
            .update(
                &[],
                "combat",
                "inn",
                SceneFamily::Interior,
                StageMode::Combat,
            )
            .unwrap();
        assert_eq!(reconnect.party_hud_transition_id.get(), 1);
        assert_eq!(
            reconnect
                .root
                .query_selector(".tv-hud")
                .unwrap()
                .unwrap()
                .get_attribute("data-party-hud")
                .as_deref(),
            Some("visible")
        );
        reconnect.dispose();
        reconnect_parent.remove();
    }

    #[test]
    fn speech_bubble_placement_keeps_off_the_title_and_rules_note() {
        assert!(BUBBLE_FIXED_OBSTACLES.contains(&".tv-headline"));
        assert!(BUBBLE_FIXED_OBSTACLES.contains(&".tv-rules"));
    }

    #[test]
    fn check_finding_has_a_full_reading_phase_after_the_roll() {
        let (short_finding_ms, short_total_ms, short_roll_ms) = check_result_phase_ms("Found.");
        let long_finding = concat!(
            "The salt-stained crest catches the lantern light beside the broken quay, revealing a courier's trail through the flooded market and a safe route beyond the watchtower before the tide turns. ",
            "Beyond the collapsed arch, wet boot marks cross the silt toward a locked storehouse, while a strip of blue cloth hangs from a rusted nail and points toward the old ferry stairs. ",
            "The marks are fresh enough to follow, but the tide is rising quickly; a careful search can recover the hidden map before the water covers the lower passage and washes away the evidence. ",
            "The courier's last message also names a bell rope above the quay, giving the party one clear landmark for the return journey if the storm closes in before dawn."
        );
        let (long_finding_ms, long_total_ms, long_roll_ms) = check_result_phase_ms(long_finding);
        assert_eq!(short_roll_ms, CHECK_ROLL_MS);
        assert_eq!(long_roll_ms, CHECK_ROLL_MS);
        assert!(long_finding_ms > short_finding_ms);
        assert_eq!(
            short_total_ms,
            short_roll_ms + CHECK_FINDING_REVEAL_MS + short_finding_ms + CHECK_FADE_MS
        );
        assert_eq!(
            long_total_ms,
            long_roll_ms + CHECK_FINDING_REVEAL_MS + long_finding_ms + CHECK_FADE_MS
        );
    }

    #[test]
    fn speech_replaces_an_active_typed_strip() {
        assert!(super::should_close_typed_strip(
            super::BubbleKind::Speech,
            true,
        ));
        assert!(!super::should_close_typed_strip(
            super::BubbleKind::Speech,
            false,
        ));
        assert!(!super::should_close_typed_strip(
            super::BubbleKind::Typed,
            true,
        ));
    }

    #[test]
    fn typed_line_anchor_matches_the_speaking_hero() {
        let heroes = [Some("Aria".to_owned()), Some("Bram".to_owned())];

        assert_eq!(typed_line_anchor("Bram", &heroes), Some(1));
    }

    #[test]
    fn typed_line_anchor_is_absent_when_the_hero_has_no_figure() {
        let heroes = [Some("Aria".to_owned()), None];

        assert_eq!(typed_line_anchor("Bram", &heroes), None);
    }

    #[test]
    fn healing_action_caption_names_the_recovered_hit_points() {
        assert_eq!(
            super::healing_action_text(super::StageMode::Combat, 6),
            "catches a second wind · +6 HP",
        );
    }
}
