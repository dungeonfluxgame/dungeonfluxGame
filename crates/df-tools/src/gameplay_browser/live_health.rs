//! Health cues on the live screens: the tier on every card and figure, the bar's lingering
//! chip and heal flash, the shake of a hit, the Critical heartbeat, and the phone's own hero
//! card, edge vignette and vibration.
//!
//! The tier is only ever `crate::health_tier::tier_of` of the numbers a view carries; sizes and
//! timings come from `crate::health_cues`. These cues play for every viewer: nothing here reads
//! a device setting that would reduce them. Every moving cue is a `translate` animation on an
//! element that does not otherwise use that property, so layout and text never move and the
//! staging transforms stay untouched. Colours, notches and labels are in `live_health.css`,
//! driven by `data-health-tier`.
use super::*;
use crate::{
    health_cues::{self, Blow, Pattern, Place, REFERENCE_STAGE_WIDTH_PX, Shake, TREMOR_PX},
    health_tier::{HealthTier, Vitals, tier_of},
};
use df_render::battle::health as battlefield;
use js_sys::{Array, Function, Object, Reflect};

/// Metres on the battlefield for each pixel of the television's 1920-pixel reference shake.
const BATTLEFIELD_METRES_PER_PIXEL: f64 = 0.01;

/// The animation name of the Critical tier's tremor, so it is started once and cancelled by
/// name.
const TREMOR_ID: &str = "health-tremor";
/// The bar's fill moves after this delay (matching `--bar-lag` in `live_health.css`), so the
/// chip starts draining after the fill has reached its new value and a short linger.
const BAR_LAG_MS: f64 = 1150.0;
const BAR_FILL_MS: f64 = 800.0;
const CHIP_LINGER_MS: f64 = 350.0;
const CHIP_DRAIN_MS: f64 = 600.0;
const HEAL_FLASH_MS: f64 = 700.0;
/// The smallest and largest scale of the television's pixel constants for the stage width.
const MIN_STAGE_SCALE: f64 = 0.3;
const MAX_STAGE_SCALE: f64 = 2.4;

fn set(object: &Object, key: &str, value: impl Into<JsValue>) -> Result<(), JsValue> {
    Reflect::set(object, &JsValue::from_str(key), &value.into()).map(|_| ())
}

fn call(target: &JsValue, method: &str, arguments: &[&JsValue]) -> Result<JsValue, JsValue> {
    let function: Function = Reflect::get(target, &JsValue::from_str(method))?.dyn_into()?;
    match arguments {
        [] => function.call0(target),
        [one] => function.call1(target, one),
        [one, two] => function.call2(target, one, two),
        _ => Err(JsValue::from_str("unsupported call arity")),
    }
}

/// Starts a Web Animation of `translate` frames on `element`.
fn animate_frames(
    element: &Element,
    frames: &[health_cues::Frame],
    amplitude_px: f64,
    duration_ms: f64,
    iterations: f64,
    id: &str,
) -> Result<(), JsValue> {
    let keyframes = Array::new();
    for (offset, x, y) in frames {
        let frame = Object::new();
        set(&frame, "offset", *offset)?;
        set(
            &frame,
            "translate",
            format!("{:.2}px {:.2}px", x * amplitude_px, y * amplitude_px),
        )?;
        keyframes.push(&frame);
    }
    let options = Object::new();
    set(&options, "duration", duration_ms)?;
    set(&options, "iterations", iterations)?;
    set(&options, "easing", "linear")?;
    set(&options, "id", id)?;
    call(element.as_ref(), "animate", &[&keyframes, &options]).map(|_| ())
}

/// Cancels the running animations of `element` called `id`.
fn cancel_named(element: &Element, id: &str) -> Result<(), JsValue> {
    let animations = call(element.as_ref(), "getAnimations", &[])?;
    for animation in Array::from(&animations).iter() {
        if Reflect::get(&animation, &JsValue::from_str("id"))?
            .as_string()
            .as_deref()
            == Some(id)
        {
            call(&animation, "cancel", &[])?;
        }
    }
    Ok(())
}

fn running(element: &Element, id: &str) -> Result<bool, JsValue> {
    let animations = call(element.as_ref(), "getAnimations", &[])?;
    Ok(Array::from(&animations).iter().any(|animation| {
        Reflect::get(&animation, &JsValue::from_str("id"))
            .ok()
            .and_then(|value| value.as_string())
            .as_deref()
            == Some(id)
    }))
}

/// Every element under `root` matching `selector`.
fn select_all(root: &Element, selector: &str) -> Result<Vec<Element>, JsValue> {
    let found = call(
        root.as_ref(),
        "querySelectorAll",
        &[&JsValue::from_str(selector)],
    )?;
    Ok(Array::from(&found)
        .iter()
        .filter_map(|node| node.dyn_into::<Element>().ok())
        .collect())
}

/// How much to scale the television's pixel constants for this stage element's width.
fn stage_scale(stage: &Element) -> f64 {
    (f64::from(stage.client_width()) / REFERENCE_STAGE_WIDTH_PX)
        .clamp(MIN_STAGE_SCALE, MAX_STAGE_SCALE)
}

/// Plays one shake on `element`, its pixels multiplied by `scale`.
fn shake(element: &Element, cue: Shake, scale: f64) -> Result<(), JsValue> {
    animate_frames(
        element,
        cue.pattern.frames(),
        cue.amplitude_px * scale,
        f64::from(cue.duration_ms),
        1.0,
        "health-shake",
    )
}

/// Sets or clears `data-health-tier`, the one attribute every cue and colour keys on.
pub(super) fn set_tier(element: &Element, tier: Option<HealthTier>) -> Result<(), JsValue> {
    match tier {
        Some(tier) => element.set_attribute("data-health-tier", tier.label()),
        None => element.remove_attribute("data-health-tier"),
    }
}

/// The tier of a character with these hit points on the display, and `Down` for one the view
/// marks down.
pub(super) fn display_tier(hit_points: Option<(u32, u32)>, down: bool) -> Option<HealthTier> {
    let (current, maximum) = hit_points?;
    tier_of(Vitals {
        current,
        maximum,
        unconscious: down,
        dying: false,
    })
}

/// Keeps the Critical tier's slow heartbeat tremor running on `element` exactly while `tier` is
/// Critical.
pub(super) fn sync_tremor(element: &Element, tier: Option<HealthTier>) -> Result<(), JsValue> {
    let tremor = health_cues::tremor();
    let active = running(element, TREMOR_ID)?;
    if tier == Some(HealthTier::Critical) {
        if !active {
            animate_frames(
                element,
                Pattern::Tremor.frames(),
                TREMOR_PX,
                f64::from(tremor.duration_ms),
                f64::INFINITY,
                TREMOR_ID,
            )?;
        }
        Ok(())
    } else if active {
        cancel_named(element, TREMOR_ID)
    } else {
        Ok(())
    }
}

/// Adds the lingering chip to a card's bar, beneath the fill.
pub(super) fn mount_bar(document: &Document, bar: &Element) -> Result<(), JsValue> {
    let chip = document.create_element("span")?;
    chip.set_class_name("tv-fig-hp-chip");
    bar.insert_before(&chip, bar.first_child().as_ref())?;
    Ok(())
}

fn percent(current: u32, maximum: u32) -> f64 {
    f64::from(current.min(maximum)) * 100.0 / f64::from(maximum.max(1))
}

/// A bar's hit points changed: on damage the lost segment lingers in the chip colour and then
/// drains; on healing the bar flashes green. The numbers and the fill are set by the caller.
pub(super) fn bar_changed(
    bar: &Element,
    before: Option<u32>,
    now: (u32, u32),
) -> Result<(), JsValue> {
    let Some(before) = before else {
        return Ok(());
    };
    let (current, maximum) = now;
    if current < before {
        if let Some(chip) = bar.query_selector(".tv-fig-hp-chip")? {
            let lingers = BAR_LAG_MS + BAR_FILL_MS + CHIP_LINGER_MS;
            let total = lingers + CHIP_DRAIN_MS;
            let from = format!("{:.1}%", percent(before, maximum));
            let to = format!("{:.1}%", percent(current, maximum));
            let frames = Array::new();
            for (offset, width) in [(0.0, &from), (lingers / total, &from), (1.0, &to)] {
                let frame = Object::new();
                set(&frame, "offset", offset)?;
                set(&frame, "width", width.as_str())?;
                frames.push(&frame);
            }
            let options = Object::new();
            set(&options, "duration", total)?;
            set(&options, "easing", "ease-in")?;
            call(chip.as_ref(), "animate", &[&frames, &options])?;
        }
    } else if current > before {
        let frames = Array::new();
        for (offset, glow, brightness) in [
            (0.0, "#7dffa000", 1.0),
            (0.25, "#7dffa0e6", 1.7),
            (1.0, "#7dffa000", 1.0),
        ] {
            let frame = Object::new();
            set(&frame, "offset", offset)?;
            set(&frame, "boxShadow", format!("0 0 22px 5px {glow}"))?;
            set(&frame, "filter", format!("brightness({brightness})"))?;
            frames.push(&frame);
        }
        let options = Object::new();
        set(&options, "duration", HEAL_FLASH_MS)?;
        set(&options, "easing", "ease-out")?;
        call(bar.as_ref(), "animate", &[&frames, &options])?;
    }
    Ok(())
}

/// A blow landed on a television figure and its card: shakes both by the share of the bar it
/// took, and the whole stage for a critical hit or a drop to 0.
pub(super) fn display_blow(stage: &Element, parts: &[&Element], blow: Blow) -> Result<(), JsValue> {
    let scale = stage_scale(stage);
    if let Some(each) = health_cues::blow_shake(blow, Place::Display) {
        for part in parts {
            shake(part, each, scale)?;
        }
    }
    if let Some(whole) = health_cues::screen_shake(blow) {
        shake(stage, whole, scale)?;
    }
    Ok(())
}

/// The crest of a card: the generated health face (cropped to the head) when the server has
/// published one for this character, otherwise the authored portrait.
pub(super) fn crest(crest: &Element, portrait: &str, live: Option<&str>) -> Result<(), JsValue> {
    match live {
        Some(url) => {
            crest.set_attribute("style", &format!("background-image:url({url})"))?;
            crest.set_attribute("data-crop", "face")
        }
        None => {
            crest.remove_attribute("data-crop")?;
            let style = if portrait.is_empty() {
                String::new()
            } else {
                format!("background-image:url({portrait})")
            };
            crest.set_attribute("style", &style)
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The phone: its own hero card, the edge vignette and the vibration.

/// What the phone remembers between two views of its own hero.
#[derive(Default)]
struct PhoneMemory {
    name: String,
    hit_points: Option<(u32, bool)>,
}

thread_local! {
    static PHONE: RefCell<PhoneMemory> = RefCell::new(PhoneMemory::default());
}

/// The phone's hero cards: the combat header and the inline sheet's header.
const PHONE_CARDS: &str = ".live-hero-head, #hero-sheet .live-sheet-head";

fn vibrate(milliseconds: u32) {
    // Only with sound and haptics on, and only for the player's own hero.
    // The sound control's mute (where the page has one) also silences the buzz; a phone has no
    // voice director of its own, so there it buzzes unless sound was muted on this page.
    if super::live_voice::effects_muted() == Some(true) {
        return;
    }
    if let Some(window) = web_sys::window() {
        let navigator: JsValue = window.navigator().into();
        // A device without vibration, or one that refuses, simply does not buzz.
        let _unsupported = call(
            &navigator,
            "vibrate",
            &[&JsValue::from_f64(f64::from(milliseconds))],
        );
    }
}

fn vignette(root: &Element, tier: Option<HealthTier>) -> Result<(), JsValue> {
    let existing = root.query_selector(":scope > .live-health-vignette")?;
    let wanted = tier.filter(|tier| matches!(tier, HealthTier::Bloodied | HealthTier::Critical));
    match (existing, wanted) {
        (Some(element), Some(tier)) => element.set_attribute("data-health-tier", tier.label()),
        (Some(element), None) => {
            element.remove();
            Ok(())
        }
        (None, Some(tier)) => {
            let document = root.owner_document().ok_or("document absent")?;
            let element = document.create_element("div")?;
            element.set_class_name("live-health-vignette");
            element.set_attribute("aria-hidden", "true")?;
            element.set_attribute("data-health-tier", tier.label())?;
            root.append_child(&element)?;
            Ok(())
        }
        (None, None) => Ok(()),
    }
}

/// Called after a phone screen is drawn: marks the player's own hero card with the tier, shows
/// the hero's health face, and on damage to that hero shakes the card and vibrates. Other
/// heroes' health never reaches a phone's motion or vibration.
pub(super) fn phone_sync(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let (root, view) = {
        let state = client.borrow();
        let Some(view) = state
            .views
            .as_ref()
            .and_then(|views| views.current().cloned())
        else {
            return Ok(());
        };
        (state.root.clone(), view)
    };
    let Some(rpc::view_message::Audience::Player(player)) = &view.audience else {
        return Ok(());
    };
    let Some(journey) = player.journey.as_ref() else {
        return Ok(());
    };
    // The character faces of every hero, for the initiative strip and the hero card.
    for (index, member) in journey.party.iter().enumerate().take(2) {
        let token = format!("hero-{}", index + 1);
        let face = journey
            .live_faces
            .iter()
            .find(|face| face.name == member.name)
            .and_then(|face| crate::presentation::imagery_url(&face.sprite));
        let selector = format!("[data-portrait=\"{token}\"]");
        for node in select_all(&root, &selector)? {
            match &face {
                Some(url) => {
                    node.set_attribute("style", &format!("--asset-portrait-{token}:url({url})"))?;
                    node.set_attribute("data-crop", "face")?;
                }
                None => {
                    node.remove_attribute("style")?;
                    node.remove_attribute("data-crop")?;
                }
            }
        }
    }
    // Every bar in the initiative strip carries its tier (colour and notches; no motion).
    for actor in select_all(&root, ".combat-actor")? {
        let name = actor
            .query_selector("h3")?
            .and_then(|heading| heading.text_content())
            .unwrap_or_default();
        let tier = journey
            .combat
            .as_ref()
            .map_or(&[][..], |combat| &combat.participants[..])
            .iter()
            .chain(journey.party.iter())
            .find(|member| member.name == name && member.maximum_hit_points > 0)
            .and_then(|member| {
                display_tier(
                    Some((member.hit_points, member.maximum_hit_points)),
                    member.unconscious,
                )
            });
        set_tier(&actor, tier)?;
        if let Some(bar) = actor.query_selector(".combat-hp")? {
            set_tier(&bar, tier)?;
        }
    }
    let Some(own) = journey.own_character.as_ref() else {
        return vignette(&root, None);
    };
    let down = journey
        .party
        .iter()
        .any(|member| member.name == own.name && member.unconscious);
    let tier = display_tier(
        (own.maximum_hit_points > 0).then_some((own.hit_points, own.maximum_hit_points)),
        down,
    );
    let elements = select_all(&root, PHONE_CARDS)?;
    for card in &elements {
        set_tier(card, tier)?;
        sync_tremor(card, tier)?;
        if let Some(bar) = card.query_selector(".combat-hp")? {
            set_tier(&bar, tier)?;
        }
    }
    vignette(&root, tier)?;
    // Compare with what this phone last showed of its own hero.
    let before = PHONE.with(|memory| {
        let mut memory = memory.borrow_mut();
        let known = (memory.name == own.name)
            .then_some(memory.hit_points)
            .flatten();
        memory.name.clone_from(&own.name);
        memory.hit_points = Some((own.hit_points, down));
        known
    });
    if let Some((previous, was_down)) = before
        && own.maximum_hit_points > 0
        && (own.hit_points < previous || (down && !was_down))
    {
        let critical = journey.combat.as_ref().is_some_and(|combat| {
            combat
                .outcomes
                .iter()
                .rev()
                .find(|outcome| outcome.target_name == own.name)
                .is_some_and(|outcome| outcome.critical && outcome.hit)
        });
        let blow = Blow {
            lost: previous.saturating_sub(own.hit_points),
            maximum: own.maximum_hit_points,
            critical,
            dropped: own.hit_points == 0 || (down && !was_down),
        };
        if let Some(each) = health_cues::blow_shake(blow, Place::Phone) {
            for card in &elements {
                shake(card, each, 1.0)?;
            }
        }
        vibrate(health_cues::vibration_ms(blow));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The 3D battlefield: its plates, figures and reactions take the same tier and the same shake
// sizes. The renderer owns no threshold: it is handed the classifier, the reaction and the
// tremor, and the server-made variants by combatant name.

fn battlefield_tone(tier: HealthTier) -> battlefield::Tone {
    match tier {
        HealthTier::Healthy => battlefield::Tone::Healthy,
        HealthTier::Hurt => battlefield::Tone::Hurt,
        HealthTier::Bloodied => battlefield::Tone::Bloodied,
        HealthTier::Critical => battlefield::Tone::Critical,
        HealthTier::Down => battlefield::Tone::Down,
    }
}

fn battlefield_reaction(cue: Shake, looping: bool) -> battlefield::Reaction {
    battlefield::Reaction {
        frames: cue.pattern.frames(),
        amplitude_m: (cue.amplitude_px * BATTLEFIELD_METRES_PER_PIXEL) as f32,
        duration_s: cue.duration_ms as f32 / 1000.0,
        looping,
    }
}

/// Hands the battlefield the shared tier function and the shake model. Called once per page.
pub(super) fn install_battlefield() {
    battlefield::install(battlefield::Hooks {
        tone: |current, maximum, down| {
            display_tier(Some((current, maximum)), down).map(battlefield_tone)
        },
        reaction: |lost, maximum, critical, dropped| {
            health_cues::blow_shake(
                Blow {
                    lost,
                    maximum,
                    critical,
                    dropped,
                },
                Place::Display,
            )
            .map(|cue| battlefield_reaction(cue, false))
        },
        tremor: battlefield_reaction(health_cues::tremor(), true),
    });
}

/// Publishes the health variants of this display view for the battlefield, by combatant name:
/// the heroes' (the art their seat wears in the fight) and the cast's.
pub(super) fn publish_battlefield_variants(
    journey: &rpc::JourneyView,
    scene: Option<&rpc::GameplayScene>,
) {
    let heroes = journey.live_faces.iter().filter_map(|face| {
        let path = crate::presentation::imagery_url(&face.battle_sprite)?;
        Some((face.name.clone(), path))
    });
    let cast = scene.into_iter().flat_map(|scene| {
        scene.cast.iter().filter_map(|actor| {
            let path = crate::presentation::imagery_url(&actor.live_sprite)?;
            Some((actor.name.clone(), path))
        })
    });
    battlefield::set_variants(heroes.chain(cast));
}
