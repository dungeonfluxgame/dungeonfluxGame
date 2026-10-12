//! The display's composited stage: the characters standing in the scene painting.
//!
//! The painting is the back layer; each character is an independent element above it (the
//! transparent cut-out, falling back to a feathered 2:3 portrait and finally to the small
//! portrait), with a contact shadow, a nameplate with a hit-point bar and speech bubbles that
//! point at the figure. Everything shown comes from the accepted view's cast; the effects
//! (step forward, shake, lunge, heal shimmer, floating numbers) are CSS animations on the
//! figure elements, started by `live_motion` and `update` and never owning game state.
//!
//! The persistent elements are updated in place. Animations that must restart (a second hit,
//! a new line) alternate between two keyframe names through a `data-fx-tick` flag, so no
//! layout is read and no timer runs.
use super::live_sfx::{self, Cue};
use super::*;
use crate::{
    presentation::{ActorSlot, SceneFamily, StageActor},
    stage_art,
};
use std::{cell::Cell, time::Duration};
use web_sys::Event;

const GENERATED: &str = live_tv::GENERATED;
const SLOTS: usize = 3;
/// Hit-point fraction, in percent, under which a figure breathes heavier and shows a vignette.
const LOW_HEALTH_PERCENT: u32 = 35;
/// An action call-out stays this long, plus a little for long text.
const ACTION_HOLD_MS: u64 = 5500;
const HOLD_PER_EXTRA_CHARACTER_MS: u64 = 40;
const MAXIMUM_ACTION_HOLD_MS: u64 = 8500;
const FADE_OUT_MS: u64 = 500;
const BUBBLE_OUT_ANIMATION: &str = "df-bubble-out";
const FLOAT_ANIMATION: &str = "df-float-up";

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
    /// A character's line: stays attributed until replaced.
    Speech,
    /// A call-out of what a character just did: fades after a few seconds.
    Action,
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
        ActorSlot::Hero(position) if position < 2 => Some(position),
        ActorSlot::Hero(_) => None,
        ActorSlot::Npc => Some(2),
    }
}

fn slot_name(position: usize) -> &'static str {
    ["hero-1", "hero-2", "npc"][position]
}

/// Where each standing place sits across the screen, as a fraction of the width.
fn slot_position(position: usize) -> f32 {
    [0.15, 0.87, 0.68][position]
}

/// The art a figure tries in order: the cut-out, the 2:3 portrait, the small portrait.
struct ArtState {
    candidates: Vec<(&'static str, String)>,
    next: usize,
}

struct Figure {
    root: Element,
    image: Element,
    name: Element,
    role: Element,
    hit_points: Element,
    hit_points_text: Element,
    bubbles: Element,
    effects: Element,
    art: Rc<RefCell<ArtState>>,
    error: Closure<dyn FnMut(Event)>,
    last_hit_points: Cell<Option<u32>>,
    name_text: RefCell<String>,
}

pub(super) struct Figures {
    root: Element,
    figures: Vec<Figure>,
    animation_end: Closure<dyn FnMut(Event)>,
    scene: RefCell<String>,
    remembered: RefCell<Option<StageActor>>,
    document: Document,
}

fn art_candidates(portrait: &str) -> Vec<(&'static str, String)> {
    let stem = stage_art::stem_of(portrait);
    let mut candidates = Vec::new();
    if let Some(sprite) = stage_art::sprite_stem(stem) {
        candidates.push(("sprite", format!("{GENERATED}/{sprite}")));
    }
    if stem.starts_with("portrait-") {
        candidates.push(("portrait", format!("{GENERATED}/{stem}-full")));
    }
    candidates.push(("card", portrait.to_owned()));
    candidates
}

fn use_candidate(figure: &Element, image: &Element, kind: &str, url: &str) -> Result<(), JsValue> {
    figure.set_attribute("data-art", kind)?;
    image.set_attribute("src", url)
}

impl Figures {
    pub(super) fn mount(document: &Document, parent: &Element) -> Result<Self, JsValue> {
        let root = node(document, parent, "div", "tv-figures", "")?;
        root.set_attribute("aria-hidden", "true")?;
        let mut figures = Vec::with_capacity(SLOTS);
        for position in 0..SLOTS {
            let figure = node(document, &root, "figure", "tv-fig", "")?;
            figure.set_attribute("data-slot", slot_name(position))?;
            figure.set_attribute("data-present", "false")?;
            let bubbles = node(document, &figure, "div", "tv-fig-bubbles", "")?;
            let body = node(document, &figure, "div", "tv-fig-body", "")?;
            node(document, &body, "div", "tv-fig-shadow", "")?;
            let image = node(document, &body, "img", "tv-fig-sprite", "")?;
            image.set_attribute("alt", "")?;
            image.set_attribute("decoding", "async")?;
            let effects = node(document, &body, "div", "tv-fig-fx", "")?;
            let plate = node(document, &figure, "div", "tv-fig-plate", "")?;
            let name = node(document, &plate, "div", "tv-fig-name", "")?;
            let role = node(document, &plate, "div", "tv-fig-role", "")?;
            let hit_points = node(document, &plate, "div", "tv-fig-hp", "")?;
            node(document, &hit_points, "span", "tv-fig-hp-fill", "")?;
            let hit_points_text = node(document, &hit_points, "span", "tv-fig-hp-text", "")?;
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
                image,
                name,
                role,
                hit_points,
                hit_points_text,
                bubbles,
                effects,
                art,
                error,
                last_hit_points: Cell::new(None),
                name_text: RefCell::new(String::new()),
            });
        }
        let animation_end = Closure::wrap(Box::new(move |event: Event| {
            let finished = js_sys::Reflect::get(&event, &JsValue::from_str("animationName"))
                .ok()
                .and_then(|name| name.as_string())
                .is_some_and(|name| name == BUBBLE_OUT_ANIMATION || name == FLOAT_ANIMATION);
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
            figures,
            animation_end,
            scene: RefCell::new(String::new()),
            remembered: RefCell::new(None),
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

    /// Shows the cast of the accepted view. A non-player character who has spoken stays on
    /// stage, no longer speaking, until the scene changes.
    pub(super) fn update(
        &self,
        cast: &[StageActor],
        scene: &str,
        family: SceneFamily,
        mode: StageMode,
    ) -> Result<(), JsValue> {
        self.root.set_attribute("data-grade", family.name())?;
        self.root.set_attribute(
            "data-mode",
            match mode {
                StageMode::Scene => "scene",
                StageMode::Combat => "combat",
                StageMode::Aftermath => "aftermath",
            },
        )?;
        if *self.scene.borrow() != scene {
            *self.scene.borrow_mut() = scene.to_owned();
            *self.remembered.borrow_mut() = None;
            self.clear_bubbles()?;
        }
        if let Some(npc) = cast.iter().find(|actor| actor.slot == ActorSlot::Npc) {
            *self.remembered.borrow_mut() = Some(npc.clone());
        }
        let remembered = self.remembered.borrow().clone().map(|mut npc| {
            if !cast.iter().any(|actor| actor.slot == ActorSlot::Npc) {
                npc.speaking = false;
            }
            npc
        });
        let mut lit = false;
        for (position, figure) in self.figures.iter().enumerate() {
            let actor = cast
                .iter()
                .find(|actor| index(actor.slot) == Some(position) && actor.slot != ActorSlot::Npc)
                .or(if position == 2 {
                    remembered.as_ref()
                } else {
                    None
                });
            match actor {
                Some(actor) => {
                    lit |= actor.speaking;
                    self.apply(figure, actor, mode)?;
                }
                None => {
                    figure.root.set_attribute("data-present", "false")?;
                    figure.last_hit_points.set(None);
                }
            }
        }
        if lit {
            self.root.set_attribute("data-lit", "")
        } else {
            self.root.remove_attribute("data-lit")
        }
    }

    fn apply(&self, figure: &Figure, actor: &StageActor, mode: StageMode) -> Result<(), JsValue> {
        let root = &figure.root;
        root.set_attribute("data-present", "true")?;
        flag(root, "data-speaking", actor.speaking)?;
        flag(root, "data-down", actor.down)?;
        if *figure.name_text.borrow() != actor.name {
            figure.name.set_text_content(Some(&actor.name));
            *figure.name_text.borrow_mut() = actor.name.clone();
        }
        figure.role.set_text_content(Some(&actor.role));
        match actor.hit_points {
            Some((current, maximum)) => {
                let percent = u64::from(current.min(maximum)) * 100 / u64::from(maximum.max(1));
                figure.hit_points.remove_attribute("hidden")?;
                figure
                    .hit_points
                    .set_attribute("style", &format!("--hp:{percent}%"))?;
                figure
                    .hit_points_text
                    .set_text_content(Some(&format!("{current} / {maximum} HP")));
                flag(
                    root,
                    "data-low",
                    percent < u64::from(LOW_HEALTH_PERCENT) && current > 0,
                )?;
                if let Some(before) = figure.last_hit_points.get()
                    && current > before
                    && index(actor.slot).is_some_and(|position| position < 2)
                {
                    self.heal(figure, actor, current - before, mode)?;
                }
                figure.last_hit_points.set(Some(current));
            }
            None => {
                figure.hit_points.set_attribute("hidden", "")?;
                root.remove_attribute("data-low")?;
                figure.last_hit_points.set(None);
            }
        }
        self.choose_art(figure, &actor.portrait)
    }

    /// Restarts the art chain only when the character changed.
    fn choose_art(&self, figure: &Figure, portrait: &str) -> Result<(), JsValue> {
        if figure.root.get_attribute("data-portrait").as_deref() == Some(portrait) {
            return Ok(());
        }
        figure.root.set_attribute("data-portrait", portrait)?;
        let mut candidates = art_candidates(portrait);
        let (kind, url) = candidates.remove(0);
        {
            let mut state = figure.art.borrow_mut();
            state.candidates = candidates;
            state.next = 0;
        }
        use_candidate(&figure.root, &figure.image, kind, &url)
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
            figure.hit_points.set_attribute("style", "--hp:0%")?;
        }
        Ok(())
    }

    fn figure(&self, slot: ActorSlot) -> Option<&Figure> {
        index(slot).and_then(|position| self.figures.get(position))
    }

    /// The standing place of the present character called `name`.
    pub(super) fn slot_named(&self, name: &str) -> Option<ActorSlot> {
        self.figures
            .iter()
            .enumerate()
            .find_map(|(position, figure)| {
                let present = figure.root.get_attribute("data-present").as_deref() == Some("true");
                (present && *figure.name_text.borrow() == name).then_some(match position {
                    0 => ActorSlot::Hero(0),
                    1 => ActorSlot::Hero(1),
                    _ => ActorSlot::Npc,
                })
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
        // Newest replaces oldest for the same figure.
        figure.bubbles.set_text_content(None);
        let bubble = node(&self.document, &figure.bubbles, "div", "tv-bubble", "")?;
        bubble.set_attribute(
            "data-kind",
            if kind == BubbleKind::Speech {
                "speech"
            } else {
                "action"
            },
        )?;
        if voiced {
            bubble.set_attribute("data-voice-role", "npc")?;
        }
        let appear_ms = appear.as_millis() as u64;
        let extra = (text.chars().count() as u64).saturating_sub(40) * HOLD_PER_EXTRA_CHARACTER_MS;
        let hold = (ACTION_HOLD_MS + extra).min(MAXIMUM_ACTION_HOLD_MS);
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
        Ok(())
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

    fn clear_bubbles(&self) -> Result<(), JsValue> {
        for figure in &self.figures {
            figure.bubbles.set_text_content(None);
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
