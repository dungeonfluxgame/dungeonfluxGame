//! The shared display's 16:9 stage for the lobby and the exploration scenes.
//!
//! One persistent DOM is built on first use and updated in place for every accepted view;
//! it holds no game state and sends nothing. Generated art is an alternate `url()` layered
//! above the authored fallback, so a missing file shows the fallback.
//!
//! Stable shell for other screens (combat): `.tv-stage` is the fixed full-viewport root,
//! `.tv-art` the full-bleed painting, `.tv-scrim` the text gradients, `.tv-safe` the
//! overscan-safe grid (rows: top, headline, spacer, content, party bar), `.tv-title` the scene
//! title with its `.tv-place` line, `.tv-stagebox` the Dungeon Master's caption and its
//! `.tv-rules` line, `.tv-choices` the read-only row of what the table can do and
//! `.tv-bigcard` the title and closing cards the director shows over the dimmed painting.
use super::live_sfx::{self, Cue};
use super::live_stage::{Figures, StageMode};
use super::*;
use crate::{
    plate_layout::{PlateFrame, Rect},
    presentation::{
        Card, CardKind, HERO_ROLE, PlateState, PlateView, SceneFamily, StageActor, asset_url,
        shot_of,
    },
    stage_machines::LineState,
};
use df_observe::presentation::{VisualPlateState, VisualShot, VisualSource};
use std::{
    cell::{Cell, RefCell},
    net::{IpAddr, SocketAddr},
    rc::{Rc, Weak},
    time::Duration,
};
use wasm_bindgen::JsCast;
use web_sys::{Event, HtmlImageElement};

#[path = "live_scene_stage.rs"]
mod live_scene_stage;
pub(super) use live_scene_stage::SceneGraphView;

pub(super) const GENERATED: &str = "/assets/ui/generated";
// PLAY-Q01: the server's QR code of the join link (see gameplay/join_link.rs).
const JOIN_QR: &str = "/gameplay/join-qr.svg";
/// The party cards a scene shows.
const PARTY_SLOTS: usize = 2;
/// The seats the lobby row always shows (open seats pad a smaller party), and the most it draws,
/// all in one row.
const LOBBY_SEATS_MIN: usize = 4;
const PARTY_MAX: usize = 6;
const GATHERED_SOUND_DELAY: Duration = Duration::from_millis(700);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TvMode {
    Lobby,
    Scene,
}

pub(super) struct TvMember {
    pub(super) name: String,
    pub(super) status: String,
    pub(super) ready: bool,
}

pub(super) struct TvSpeaker {
    pub(super) name: String,
    pub(super) line: String,
    pub(super) portrait: Option<String>,
}

/// Everything the stage shows, already chosen from the accepted server view.
pub(super) struct TvModel {
    pub(super) mode: TvMode,
    pub(super) title: String,
    pub(super) lede: String,
    pub(super) speaker: Option<TvSpeaker>,
    pub(super) room_code: String,
    // PLAY-Q01: the host the room code is typed at, named in the join hint.
    pub(super) join_host: String,
    pub(super) party: Vec<TvMember>,
    pub(super) art: String,
    pub(super) art_fallback: &'static str,
    pub(super) art_source: VisualSource,
    pub(super) cast: Vec<StageActor>,
    /// The place the scene is set in, shown under its title.
    pub(super) place: String,
    /// The rules wording of the scene, kept apart from the story narration.
    pub(super) rules_note: String,
    /// What the table can do now: labels only, for the read-only row of chips.
    pub(super) choices: Vec<String>,
    /// The table's setting: the chip row is hidden and the narrator box takes its room.
    pub(super) choice_hints_hidden: bool,
    pub(super) family: SceneFamily,
    /// Names the scene for the figures: a non-player character stays until it changes.
    pub(super) scene_key: String,
    /// Stable scene id shared with the server's content-free plate timeline.
    pub(super) visual_scene_id: String,
    /// The lobby's home screen from the server; absent outside the lobby and for a fallback.
    pub(super) home: Option<rpc::HomeTheme>,
    /// PLAY-V10: the served loop of the lobby's home painting, once the server says it is ready.
    pub(super) home_video: Option<String>,
    /// PLAY-V09: the scene's composed plate, which replaces the painting and the cut-outs.
    pub(super) plate: PlateView,
    /// PLAY-V12: the adventure's own title, for the establishing shot's title card.
    pub(super) adventure_name: String,
    /// PLAY-V12: the scene has narration, which plays over its establishing shot.
    pub(super) narrated: bool,
    /// The adventure's end card, shown over the scene and kept up until the next step.
    pub(super) end: Option<crate::presentation::Card>,
    /// The public result of the latest ability check, staged beside its hero.
    pub(super) check: Option<live_stage::CheckDisplay>,
}

struct WordmarkSlot {
    image: Element,
    text: Element,
    load: Closure<dyn FnMut(Event)>,
}

struct SeatCard {
    root: Element,
    glyph: Element,
    portrait: Element,
    name: Element,
    status: Element,
}

/// The centred card of the story's title and of a fight's close.
struct BigCard {
    root: Element,
    kicker: Element,
    headline: Element,
    line: Element,
    place: Element,
    heroes: Element,
    rewards: Element,
}

struct SoundGate {
    layer: Element,
    start: Element,
    skip: Element,
    dismiss_listeners: Vec<Closure<dyn FnMut(Event)>>,
    skip_guards: Vec<Closure<dyn FnMut(Event)>>,
}

/// PLAY-V09: the layer that shows the scene plate: two pictures (the shown one and the one that is
/// loading or leaving), a slow push-in, drifting dust and a breathing vignette. The stylesheet
/// (`live_plate.css`) draws every state from the attributes set here.
type PictureListeners = (Closure<dyn FnMut(Event)>, Closure<dyn FnMut(Event)>);
type FrameListener = Closure<dyn FnMut(Event)>;

struct PlateLayer {
    root: Element,
    frames: Vec<Element>,
    images: Vec<Element>,
    video_loops: Rc<RefCell<Vec<live_home_video::HomeLoop>>>,
    video_paths: Rc<RefCell<Vec<Option<String>>>>,
    /// Each picture's `load` and `error` listeners, kept so they can be removed.
    listeners: Vec<PictureListeners>,
    frame_listeners: Vec<FrameListener>,
    video_ready_listeners: Vec<(Element, &'static str, FrameListener)>,
    /// The scene whose plate is shown, to tell an edit of it from the plate of a new scene.
    scene: RefCell<String>,
    /// Which of the two pictures is the next to be filled.
    next: Cell<usize>,
    /// Distinguishes callbacks from successive requests that reuse the inactive picture.
    request: Cell<u64>,
    /// The scene the last view was for, so a new scene can be told from the one on screen.
    seen: RefCell<String>,
    /// Plate pictures fetched ahead, by path, so a scene change can show its plate at once.
    preloads: RefCell<Vec<(String, Element)>>,
    /// Layout to apply only when its matching picture becomes active.
    pending_layout: Rc<RefCell<Option<crate::presentation::PlateGeometry>>>,
    /// Intrinsic dimensions of the active picture, used to mirror `object-fit: cover`.
    picture_size: Rc<Cell<Option<(f64, f64)>>>,
}

/// How many plate pictures are kept fetched ahead.
const MAXIMUM_PRELOADS: usize = 6;

/// PLAY-V12: the speaker's portrait, name and line in a lower third, drawn over the establishing
/// shot while a character talks and the plate is not here yet.
struct LowerThird {
    root: Element,
    glyph: Element,
    portrait: Element,
    name: Element,
    text: Element,
}

pub(super) struct TvStage {
    root: Element,
    art: Element,
    art_previous: Element,
    plate: PlateLayer,
    scene_graph: live_scene_stage::StageLayer,
    /// PLAY-V12: the establishing shot's title card: the adventure title on opening, then the scene title.
    establish_title: Element,
    establish_place: Element,
    lower: LowerThird,
    /// The portraits of the characters on stage, by name, for the lower third.
    portraits: RefCell<Vec<(String, String)>>,
    /// The newest narrator line the stage has heard of, and the one the current scene's entry
    /// began after: only a later line is this scene's narration.
    last_narrator_line: Cell<u64>,
    narration_floor: Cell<u64>,
    figures: Rc<Figures>,
    roster: Element,
    gathered: Element,
    /// Whether a view has been drawn yet: the first one plays no seat sounds.
    drawn: Cell<bool>,
    /// The scene just drawn is set in the place of the one before it.
    same_place: Cell<bool>,
    title: Element,
    place: Element,
    lede: Element,
    atmosphere: Element,
    /// PLAY-V10: the looping video over the still, below the scrim and the themed blocks.
    home_loop: live_home_video::HomeLoop,
    lobby_row: Element,
    stagebox: Element,
    narration: Element,
    rules: Element,
    rules_text: Element,
    choices: Element,
    card: BigCard,
    speaker: Element,
    speaker_glyph: Element,
    speaker_portrait: Element,
    speaker_name: Element,
    speaker_line: Element,
    code: Element,
    // PLAY-Q01: the join hint, which names the host for the room code.
    hint: Element,
    roster_slot: Element,
    party_bar: Element,
    party: Element,
    cards: Vec<SeatCard>,
    dock: Element,
    wordmark: WordmarkSlot,
    sound_gate: SoundGate,
}

/// The loading wordmark, centred with a subtle shimmer until the first view arrives.
pub(super) fn mount_splash(document: &Document, parent: &Element) -> Result<(), JsValue> {
    let splash = node(document, parent, "div", "tv-splash", "")?;
    node(document, &splash, "div", "tv-splash-mark", "")?;
    Ok(())
}

/// A live image leads its reviewed fallback; an absent image leaves the stage's base color.
pub(super) fn image_layers(stem: &str, fallback: &str) -> String {
    match (stem.is_empty(), fallback.is_empty()) {
        (true, true) => "background-image:none".to_owned(),
        (true, false) => format!("background-image:url({fallback})"),
        (false, true) => format!("background-image:url({stem})"),
        (false, false) => format!("background-image:url({stem}),url({fallback})"),
    }
}

fn set_hidden(element: &Element, hidden: bool) -> Result<(), JsValue> {
    if hidden {
        element.set_attribute("hidden", "")
    } else {
        element.remove_attribute("hidden")
    }
}

fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map_or_else(|| "?".to_owned(), |first| first.to_uppercase().collect())
}

fn is_loopback_join_host(join_host: &str) -> bool {
    if join_host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if join_host
        .parse::<SocketAddr>()
        .is_ok_and(|address| address.ip().is_loopback())
    {
        return true;
    }
    if join_host
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_loopback())
    {
        return true;
    }
    join_host
        .split_once(':')
        .is_some_and(|(hostname, _)| hostname.eq_ignore_ascii_case("localhost"))
}

fn portrait(
    document: &Document,
    parent: &Element,
    class: &str,
) -> Result<(Element, Element, Element), JsValue> {
    let frame = node(document, parent, "div", class, "")?;
    let glyph = node(document, &frame, "span", "tv-portrait-glyph", "")?;
    let image = node(document, &frame, "div", "tv-portrait-img", "")?;
    Ok((frame, glyph, image))
}

fn mount_card(document: &Document, parent: &Element) -> Result<BigCard, JsValue> {
    let root = node(document, parent, "div", "tv-bigcard", "")?;
    root.set_attribute("aria-hidden", "true")?;
    let kicker = node(document, &root, "p", "tv-bigcard-kicker", "")?;
    let headline = node(document, &root, "h2", "tv-bigcard-title", "")?;
    node(document, &root, "div", "tv-bigcard-rule", "")?;
    let line = node(document, &root, "p", "tv-bigcard-line", "")?;
    let place = node(document, &root, "p", "tv-bigcard-place", "")?;
    let heroes = node(document, &root, "ul", "tv-bigcard-heroes", "")?;
    let rewards = node(document, &root, "div", "tv-bigcard-rewards", "")?;
    set_hidden(&rewards, true)?;
    // The next-step prompt of the end card belongs here; it stays empty until that step exists.
    let next = node(document, &root, "p", "tv-bigcard-next", "")?;
    next.set_attribute("data-slot", "next-step")?;
    set_hidden(&next, true)?;
    Ok(BigCard {
        root,
        kicker,
        headline,
        line,
        place,
        heroes,
        rewards,
    })
}

fn mount_sound_gate(document: &Document, stage: &Element) -> Result<SoundGate, JsValue> {
    let layer = node(document, stage, "section", "tv-sound-gate", "")?;
    layer.set_attribute("role", "dialog")?;
    layer.set_attribute("aria-modal", "true")?;
    layer.set_attribute("aria-labelledby", "tv-sound-title")?;
    let title = node(document, &layer, "h1", "tv-sound-title", "Start with sound")?;
    title.set_attribute("id", "tv-sound-title")?;
    node(
        document,
        &layer,
        "p",
        "tv-sound-copy",
        "Hear the story together. You can mute sound at any time.",
    )?;
    let controls = node(document, &layer, "div", "tv-sound-actions", "")?;
    let start = node(
        document,
        &controls,
        "button",
        "tv-sound-start",
        "Start with sound",
    )?;
    start.set_attribute("type", "button")?;
    let skip = node(
        document,
        &controls,
        "button",
        "tv-sound-skip",
        "Continue without sound",
    )?;
    skip.set_attribute("type", "button")?;
    skip.set_attribute("data-sound-skip", "")?;
    stage.set_attribute("data-sound-gate", "")?;

    let mut dismiss_listeners = Vec::with_capacity(2);
    for button in [&start, &skip] {
        let (stage, layer) = (stage.clone(), layer.clone());
        let listener = Closure::wrap(Box::new(move |_: Event| {
            let _removed = stage.remove_attribute("data-sound-gate");
            layer.remove();
        }) as Box<dyn FnMut(Event)>);
        button.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref())?;
        dismiss_listeners.push(listener);
    }

    let stop_unlock = Closure::wrap(Box::new(move |event: Event| {
        event.stop_propagation();
    }) as Box<dyn FnMut(Event)>);
    skip.add_event_listener_with_callback("pointerdown", stop_unlock.as_ref().unchecked_ref())?;
    skip.add_event_listener_with_callback("keydown", stop_unlock.as_ref().unchecked_ref())?;

    Ok(SoundGate {
        layer,
        start,
        skip,
        dismiss_listeners,
        skip_guards: vec![stop_unlock],
    })
}

/// PLAY-V12: sets `data-shot` (which picture a story scene shows) from the flags the plate layer
/// and the narration keep on the stage root: the plate's state, whether its picture has loaded and
/// whether the narration of the scene's entry is over. Outside a story scene the stage keeps its
/// own picture.
fn sync_shot(stage: &Element) -> Result<(), JsValue> {
    let is_scene = stage.get_attribute("data-tv-mode").as_deref() == Some("scene");
    let plate_state =
        PlateState::from_name(&stage.get_attribute("data-plate-state").unwrap_or_default());
    let shot = if is_scene {
        if stage.has_attribute("data-plate-loaded")
            && stage.get_attribute("data-shot").as_deref() == Some("plate")
        {
            crate::presentation::Shot::Plate
        } else {
            shot_of(
                plate_state,
                stage.has_attribute("data-plate-loaded"),
                stage.has_attribute("data-narration-over"),
            )
        }
    } else {
        crate::presentation::Shot::Stage
    };
    stage.set_attribute("data-shot", shot.name())?;
    if is_scene {
        record_visual_source(stage, shot, plate_state)?;
    }
    refresh_timing_visibility(stage);
    Ok(())
}

fn first_art_asset(style: &str) -> Option<&str> {
    style
        .strip_prefix("background-image:url(")?
        .split("),url(")
        .next()
        .filter(|asset| !asset.is_empty())
}

fn plate_asset_key(asset: &str) -> &str {
    asset
        .rsplit('/')
        .next()
        .filter(|key| !key.is_empty())
        .unwrap_or(asset)
}

fn record_visual_source(
    stage: &Element,
    shot: crate::presentation::Shot,
    plate_state: PlateState,
) -> Result<(), JsValue> {
    let art_source = match stage.get_attribute("data-art-source").as_deref() {
        Some("live_backdrop") => VisualSource::LiveBackdrop,
        Some("establishing_photo") => VisualSource::EstablishingPhoto,
        _ => VisualSource::SceneArtwork,
    };
    let requested = if matches!(
        plate_state,
        PlateState::Pending | PlateState::Ready | PlateState::Failed
    ) {
        VisualSource::ScenePlate
    } else {
        art_source
    };
    let effective = if shot == crate::presentation::Shot::Plate {
        VisualSource::ScenePlate
    } else {
        art_source
    };
    let shot_kind = match shot {
        crate::presentation::Shot::Plate => VisualShot::Plate,
        crate::presentation::Shot::Establishing => VisualShot::Establishing,
        crate::presentation::Shot::Stage => VisualShot::Stage,
    };
    let diagnostic_state = match plate_state {
        PlateState::None => VisualPlateState::None,
        PlateState::Pending => VisualPlateState::Pending,
        PlateState::Ready => VisualPlateState::Ready,
        PlateState::Failed => VisualPlateState::Failed,
        PlateState::Unavailable => VisualPlateState::Unavailable,
    };
    let art_asset = stage
        .query_selector(".tv-art")
        .ok()
        .flatten()
        .and_then(|art| art.get_attribute("style"))
        .as_deref()
        .and_then(first_art_asset)
        .map(str::to_owned);
    let plate_layer = stage.query_selector(".tv-plate").ok().flatten();
    let requested_asset = if requested == VisualSource::ScenePlate {
        plate_layer
            .as_ref()
            .and_then(|layer| layer.get_attribute("data-want-key"))
            .or_else(|| {
                plate_layer
                    .as_ref()
                    .and_then(|layer| layer.get_attribute("data-failed"))
            })
            .or_else(|| stage.get_attribute("data-plate-key"))
    } else {
        art_asset.clone()
    };
    let effective_asset = if effective == VisualSource::ScenePlate {
        stage.get_attribute("data-plate-key")
    } else {
        art_asset
    };
    let scene = stage
        .get_attribute("data-visual-scene-id")
        .or_else(|| stage.get_attribute("data-scene-key"))
        .unwrap_or_default();
    let requested_asset_id = requested_asset.as_deref().map(|asset| {
        super::presentation_timing::identity(if requested == VisualSource::ScenePlate {
            plate_asset_key(asset)
        } else {
            asset
        })
    });
    let effective_asset_id = effective_asset.as_deref().map(|asset| {
        super::presentation_timing::identity(if effective == VisualSource::ScenePlate {
            plate_asset_key(asset)
        } else {
            asset
        })
    });
    let signature = format!(
        "{}:{}:{}:{}:{}:{}:{}",
        super::presentation_timing::identity(&scene),
        requested.label(),
        effective.label(),
        shot_kind.label(),
        diagnostic_state.label(),
        requested_asset_id.unwrap_or_default(),
        effective_asset_id.unwrap_or_default(),
    );
    if stage
        .get_attribute("data-visual-source-signature")
        .as_deref()
        == Some(&signature)
    {
        return Ok(());
    }
    stage.set_attribute("data-visual-source-signature", &signature)?;
    super::presentation_timing::visual_source(
        &scene,
        requested,
        effective,
        shot_kind,
        diagnostic_state,
        requested_asset_id,
        effective_asset_id,
    );
    Ok(())
}

/// Observe visibility after motion-only changes as well as media/view updates.
pub(super) fn refresh_timing_visibility(stage: &Element) {
    live_scene_stage::refresh_timing_visibility(stage);
    let check_visible = stage.is_connected()
        && stage.get_attribute("data-tv-mode").as_deref() == Some("scene")
        && !stage.has_attribute("data-sound-gate")
        && stage.has_attribute("data-check-active")
        && stage.get_attribute("data-battlefield-live").as_deref() != Some("true");
    let check = if check_visible {
        stage
            .query_selector(".tv-check-result")
            .ok()
            .flatten()
            .and_then(|panel| panel.get_attribute("data-check-signature"))
    } else {
        None
    };
    let previous = stage.get_attribute("data-timing-check");
    if previous != check {
        let scene = stage.get_attribute("data-scene-key").unwrap_or_default();
        if let Some(previous) = previous {
            super::presentation_timing::media("check-hidden", &scene, &previous, "replaced", 0);
        }
        if let Some(signature) = check {
            if stage.set_attribute("data-timing-check", &signature).is_ok() {
                super::presentation_timing::media("check-shown", &scene, &signature, "ready", 0);
            }
        } else {
            let _removed = stage.remove_attribute("data-timing-check");
        }
    }
    let Ok(Some(plate)) = stage.query_selector(".tv-plate") else {
        return;
    };
    let visible = stage.is_connected()
        && stage.get_attribute("data-tv-mode").as_deref() == Some("scene")
        && stage.get_attribute("data-shot").as_deref() == Some("plate")
        && stage.get_attribute("data-battlefield-live").as_deref() != Some("true")
        && stage
            .get_attribute("data-battle-state")
            .is_none_or(|state| state == "absent")
        && !stage.has_attribute("data-sound-gate");
    let active = plate.get_attribute("data-active");
    let scene = stage.get_attribute("data-scene-key").unwrap_or_default();
    let Ok(frames) = plate.query_selector_all(".tv-plate-frame") else {
        return;
    };
    for index in 0..frames.length() {
        let Some(frame) = frames
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
        else {
            continue;
        };
        let shown = visible && active == frame.get_attribute("data-i");
        if !shown {
            let _removed = frame.remove_attribute("data-timing-shown");
            let _removed = frame.remove_attribute("data-clip-shown");
            continue;
        }
        if let Ok(Some(image)) = frame.query_selector(".tv-plate-img") {
            let request = image.get_attribute("data-request");
            if request.is_some()
                && frame.get_attribute("data-timing-shown") != request
                && let Some(request) = request
                && frame.set_attribute("data-timing-shown", &request).is_ok()
            {
                super::presentation_timing::media(
                    "plate-shown",
                    &scene,
                    &image.get_attribute("data-src").unwrap_or_default(),
                    "ready",
                    1200,
                );
            }
        }
        let playing = frame
            .query_selector_all("video.tv-loop-clip")
            .ok()
            .and_then(|videos| {
                (0..videos.length()).find_map(|index| {
                    let video = videos
                        .item(index)?
                        .dyn_into::<web_sys::HtmlVideoElement>()
                        .ok()?;
                    (!video.paused()
                        && video.has_attribute("data-on")
                        && video.has_attribute("data-timing-playing"))
                    .then_some(video)
                })
            });
        if let Some(video) = playing {
            let path = video.get_attribute("src");
            if video.has_attribute("data-timing-playing")
                && path.is_some()
                && frame.get_attribute("data-clip-shown") != path
                && let Some(path) = path
                && frame.set_attribute("data-clip-shown", &path).is_ok()
            {
                super::presentation_timing::media("clip-shown", &scene, &path, "ready", 1200);
            }
        }
    }
}

fn scene_video_url(path: &str) -> Option<&str> {
    let key = path.strip_prefix("/gameplay/home-video/")?;
    (key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(path)
}

fn check_skill_label(source_revision: &str) -> Option<(&'static str, &'static str)> {
    let parts = source_revision.splitn(4, ':').collect::<Vec<_>>();
    let ["srd521-p6", _, skill, _] = parts.as_slice() else {
        return None;
    };
    // SRD 5.2.1 p. 9, Skills: names and the ability check each skill most often applies to.
    match *skill {
        "acrobatics" => Some(("Dexterity", "Acrobatics")),
        "animal-handling" => Some(("Wisdom", "Animal Handling")),
        "athletics" => Some(("Strength", "Athletics")),
        "deception" => Some(("Charisma", "Deception")),
        "intimidation" => Some(("Charisma", "Intimidation")),
        "performance" => Some(("Charisma", "Performance")),
        "persuasion" => Some(("Charisma", "Persuasion")),
        "sleight-of-hand" => Some(("Dexterity", "Sleight of Hand")),
        "perception" => Some(("Wisdom", "Perception")),
        "insight" => Some(("Wisdom", "Insight")),
        "survival" => Some(("Wisdom", "Survival")),
        "medicine" => Some(("Wisdom", "Medicine")),
        "investigation" => Some(("Intelligence", "Investigation")),
        "arcana" => Some(("Intelligence", "Arcana")),
        "history" => Some(("Intelligence", "History")),
        "nature" => Some(("Intelligence", "Nature")),
        "religion" => Some(("Intelligence", "Religion")),
        "stealth" => Some(("Dexterity", "Stealth")),
        _ => None,
    }
}

pub(super) fn check_display(
    check: &rpc::AbilityCheckResult,
    journey: &rpc::JourneyView,
    finding: &str,
) -> Result<live_stage::CheckDisplay, JsValue> {
    // SRD 5.2.1 p. 6: add the ability and proficiency modifiers to the d20; p. 7: Advantage
    // and Disadvantage roll two d20s and keep the higher or lower face, respectively.
    let proficiency_modifier = i32::try_from(check.proficiency_bonus)
        .map_err(|_| JsValue::from_str("display check proficiency modifier is invalid"))?;
    if !(1..=20).contains(&check.die) || !matches!(check.roll_mode, -1..=1) {
        return Err(JsValue::from_str("display check roll is invalid"));
    }
    let expected_faces = if check.roll_mode == 0 { 1 } else { 2 };
    if check.faces.len() != expected_faces
        || check.faces.iter().any(|face| !(1..=20).contains(face))
    {
        return Err(JsValue::from_str("display check dice are invalid"));
    }
    let kept = check
        .faces
        .iter()
        .position(|face| *face == check.die)
        .ok_or_else(|| JsValue::from_str("display check kept die is absent"))?;
    let (ability, skill) = check_skill_label(&check.source_revision)
        .ok_or_else(|| JsValue::from_str("display check skill is unknown"))?;
    let roller_index = journey
        .table_log
        .iter()
        .rev()
        .find(|line| {
            rpc::TableRole::try_from(line.role) == Ok(rpc::TableRole::Narrator)
                && rpc::TableKind::try_from(line.kind) == Ok(rpc::TableKind::Action)
                && line.speaker == "Roll"
        })
        .map(|line| line.hero_index)
        .and_then(|index| usize::try_from(index).ok())
        .ok_or_else(|| JsValue::from_str("display check roller is absent"))?;
    let hero_name = journey
        .party
        .get(roller_index)
        .map(|hero| hero.name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| JsValue::from_str("display check roller is absent"))?;
    let modifier = check
        .ability_modifier
        .checked_add(proficiency_modifier)
        .ok_or_else(|| JsValue::from_str("display check modifier is invalid"))?;
    let finding = crate::presentation::first_sentence(finding);
    if finding.is_empty() {
        return Err(JsValue::from_str("display check finding is absent"));
    }
    Ok(live_stage::CheckDisplay {
        hero_name,
        ability_skill: format!("{ability} ({skill})"),
        faces: check
            .faces
            .iter()
            .copied()
            .enumerate()
            .map(|(index, face)| (face, index == kept))
            .collect(),
        roll_mode: check.roll_mode,
        modifier,
        total: check.total,
        succeeded: check.succeeded,
        finding,
    })
}

fn mount_lower_third(document: &Document, parent: &Element) -> Result<LowerThird, JsValue> {
    let root = node(document, parent, "div", "tv-lower", "")?;
    root.set_attribute("aria-hidden", "true")?;
    let (_, glyph, portrait) = portrait(document, &root, "tv-portrait")?;
    let words = node(document, &root, "div", "tv-lower-words", "")?;
    let name = node(document, &words, "h2", "tv-lower-name", "")?;
    let text = node(document, &words, "p", "tv-lower-text", "")?;
    Ok(LowerThird {
        root,
        glyph,
        portrait,
        name,
        text,
    })
}

/// PLAY-V09: builds the plate layer between the painting and the scrim.
fn mount_plate(document: &Document, stage: &Element) -> Result<PlateLayer, JsValue> {
    let root = node(document, stage, "div", "tv-plate", "")?;
    root.set_attribute("aria-hidden", "true")?;
    let zoom = node(document, &root, "div", "tv-plate-zoom", "")?;
    let mut frames = Vec::with_capacity(2);
    let mut images = Vec::with_capacity(2);
    let mut video_loops = Vec::with_capacity(2);
    for index in 0..2 {
        let frame = node(document, &zoom, "div", "tv-plate-frame", "")?;
        frame.set_attribute("data-i", &index.to_string())?;
        let image = node(document, &frame, "img", "tv-plate-img", "")?;
        image.set_attribute("alt", "")?;
        image.set_attribute("decoding", "async")?;
        image.set_attribute("data-i", &index.to_string())?;
        let video_layer = node(document, &frame, "div", "tv-loop tv-plate-video", "")?;
        video_loops.push(live_home_video::HomeLoop::new(document, video_layer)?);
        frames.push(frame);
        images.push(image);
    }
    node(document, &root, "div", "tv-plate-dust", "")?;
    node(document, &root, "div", "tv-plate-vignette", "")?;
    Ok(PlateLayer {
        root,
        frames,
        images,
        video_loops: Rc::new(RefCell::new(video_loops)),
        video_paths: Rc::new(RefCell::new(vec![None, None])),
        listeners: Vec::with_capacity(2),
        frame_listeners: Vec::with_capacity(2),
        video_ready_listeners: Vec::with_capacity(2),
        scene: RefCell::new(String::new()),
        next: Cell::new(0),
        request: Cell::new(0),
        seen: RefCell::new(String::new()),
        preloads: RefCell::new(Vec::new()),
        pending_layout: Rc::new(RefCell::new(None)),
        picture_size: Rc::new(Cell::new(None)),
    })
}

fn bind_plate_events(
    layer: &mut PlateLayer,
    stage: &Element,
    figures: Weak<Figures>,
) -> Result<(), JsValue> {
    for image in &layer.images {
        let load = {
            let (stage, root, image) = (stage.clone(), layer.root.clone(), image.clone());
            let figures = figures.clone();
            let pending_layout = Rc::clone(&layer.pending_layout);
            let picture_size = Rc::clone(&layer.picture_size);
            Closure::wrap(Box::new(move |_: Event| {
                if !plate_request_matches(&root, &image) {
                    return;
                }
                let Ok(image) = image.clone().dyn_into::<HtmlImageElement>() else {
                    fail_plate_request(
                        &stage,
                        &root,
                        &image,
                        &figures,
                        &pending_layout,
                        &picture_size,
                    );
                    return;
                };
                super::presentation_timing::media(
                    "plate-loaded",
                    &stage.get_attribute("data-scene-key").unwrap_or_default(),
                    &image.get_attribute("data-src").unwrap_or_default(),
                    "ready",
                    0,
                );
                let decoded = image.decode();
                let (decode_stage, decode_root, decode_figures) =
                    (stage.clone(), root.clone(), figures.clone());
                let (decode_layout, decode_picture_size) =
                    (Rc::clone(&pending_layout), Rc::clone(&picture_size));
                wasm_bindgen_futures::spawn_local(async move {
                    if wasm_bindgen_futures::JsFuture::from(decoded).await.is_ok() {
                        if plate_request_matches(&decode_root, &image)
                            && let Some(request) = decode_root.get_attribute("data-request")
                            && image.set_attribute("data-decoded", &request).is_ok()
                        {
                            super::presentation_timing::media(
                                "plate-decoded",
                                &decode_stage
                                    .get_attribute("data-scene-key")
                                    .unwrap_or_default(),
                                &image.get_attribute("data-src").unwrap_or_default(),
                                "ready",
                                0,
                            );
                        }
                        activate_plate_request(
                            &decode_stage,
                            &decode_root,
                            image.as_ref(),
                            &decode_figures,
                            &decode_layout,
                            &decode_picture_size,
                        );
                    } else {
                        fail_plate_request(
                            &decode_stage,
                            &decode_root,
                            image.as_ref(),
                            &decode_figures,
                            &decode_layout,
                            &decode_picture_size,
                        );
                    }
                });
            }) as Box<dyn FnMut(Event)>)
        };
        let error = {
            let (stage, root, image) = (stage.clone(), layer.root.clone(), image.clone());
            let figures = figures.clone();
            let pending_layout = Rc::clone(&layer.pending_layout);
            let picture_size = Rc::clone(&layer.picture_size);
            Closure::wrap(Box::new(move |_: Event| {
                fail_plate_request(
                    &stage,
                    &root,
                    &image,
                    &figures,
                    &pending_layout,
                    &picture_size,
                );
            }) as Box<dyn FnMut(Event)>)
        };
        image.add_event_listener_with_callback("load", load.as_ref().unchecked_ref())?;
        image.add_event_listener_with_callback("error", error.as_ref().unchecked_ref())?;
        layer.listeners.push((load, error));
    }
    for (index, frame) in layer.frames.iter().enumerate() {
        let (root, frame, event_frame) = (layer.root.clone(), frame.clone(), frame.clone());
        let video_frame = frame.clone();
        let video_loops = Rc::clone(&layer.video_loops);
        let video_paths = Rc::clone(&layer.video_paths);
        let listener = Closure::wrap(Box::new(move |event: Event| {
            let is_frame = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
                .is_some_and(|target| target.is_same_node(Some(&event_frame)));
            let is_opacity =
                js_sys::Reflect::get(event.as_ref(), &JsValue::from_str("propertyName"))
                    .ok()
                    .and_then(|property| property.as_string())
                    .as_deref()
                    == Some("opacity");
            let active_index = root
                .get_attribute("data-active")
                .and_then(|active| active.parse::<usize>().ok());
            let transitioning_index = root
                .get_attribute("data-transition")
                .and_then(|transitioning| transitioning.parse::<usize>().ok());
            if is_frame && is_opacity {
                if active_index == Some(index) && transitioning_index == Some(index) {
                    let _removed = root.remove_attribute("data-transition");
                    if let (Some(video_loop), Some(path)) = (
                        video_loops.borrow().get(index),
                        video_paths.borrow().get(index),
                    ) {
                        let _applied = video_loop.apply(path.as_deref());
                    }
                } else if active_index != Some(index)
                    && let Some(video_loop) = video_loops.borrow().get(index)
                {
                    video_loop.release();
                    let _removed = frame.remove_attribute("data-video-ready");
                }
            }
        }) as Box<dyn FnMut(Event)>);
        video_frame
            .add_event_listener_with_callback("transitionend", listener.as_ref().unchecked_ref())?;
        layer.frame_listeners.push(listener);

        let Some(video) = video_frame.query_selector("video.tv-loop-clip")? else {
            continue;
        };
        let timing_stage = stage.clone();
        let timing_figures = figures.clone();
        let (stage, root, frame, image) = (
            stage.clone(),
            layer.root.clone(),
            video_frame.clone(),
            layer.images[index].clone(),
        );
        let figures = figures.clone();
        let pending_layout = Rc::clone(&layer.pending_layout);
        let picture_size = Rc::clone(&layer.picture_size);
        let ready = Closure::wrap(Box::new(move |event: Event| {
            let Some(wanted_video) = frame.get_attribute("data-video-want") else {
                return;
            };
            let is_wanted_video = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
                .and_then(|target| target.get_attribute("src"))
                .as_deref()
                == Some(wanted_video.as_str());
            if !is_wanted_video {
                return;
            }
            if frame
                .set_attribute("data-video-ready", &wanted_video)
                .is_ok()
            {
                super::presentation_timing::media(
                    "clip-ready",
                    &stage.get_attribute("data-scene-key").unwrap_or_default(),
                    &wanted_video,
                    "ready",
                    0,
                );
            }
            activate_plate_request(
                &stage,
                &root,
                &image,
                &figures,
                &pending_layout,
                &picture_size,
            );
        }) as Box<dyn FnMut(Event)>);
        video.add_event_listener_with_callback("canplay", ready.as_ref().unchecked_ref())?;
        layer.video_ready_listeners.push((video, "canplay", ready));
        let videos = video_frame.query_selector_all("video.tv-loop-clip")?;
        for video_index in 0..videos.length() {
            let Some(video) = videos
                .item(video_index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            for event_name in ["playing", "ended", "error"] {
                let video = video.clone();
                let (stage, root, image) = (
                    timing_stage.clone(),
                    layer.root.clone(),
                    layer.images[index].clone(),
                );
                let figures = timing_figures.clone();
                let (layout, size) = (
                    Rc::clone(&layer.pending_layout),
                    Rc::clone(&layer.picture_size),
                );
                let observed_video = video.clone();
                let listener = Closure::wrap(Box::new(move |_: Event| {
                    let path = observed_video.get_attribute("src").unwrap_or_default();
                    let current = image
                        .parent_element()
                        .and_then(|frame| frame.get_attribute("data-video-want"));
                    if !observed_video.is_connected() || current.as_deref() != Some(&path) {
                        return;
                    }
                    match event_name {
                        "playing" => {
                            if observed_video
                                .set_attribute("data-timing-playing", "")
                                .is_ok()
                            {
                                refresh_timing_visibility(&stage);
                            }
                        }
                        "ended" => {
                            let _removed = observed_video.remove_attribute("data-timing-playing");
                            super::presentation_timing::media(
                                "clip-ended",
                                &stage.get_attribute("data-scene-key").unwrap_or_default(),
                                &path,
                                "ended",
                                0,
                            );
                        }
                        _ => {
                            let _removed = observed_video.remove_attribute("data-timing-playing");
                            super::presentation_timing::media(
                                "clip-failed",
                                &stage.get_attribute("data-scene-key").unwrap_or_default(),
                                &path,
                                "failed",
                                0,
                            );
                            fail_plate_request(&stage, &root, &image, &figures, &layout, &size);
                        }
                    }
                }) as Box<dyn FnMut(Event)>);
                // Observe failure before HomeLoop's listener releases and clears the source.
                video.add_event_listener_with_callback_and_bool(
                    event_name,
                    listener.as_ref().unchecked_ref(),
                    true,
                )?;
                layer
                    .video_ready_listeners
                    .push((video, event_name, listener));
            }
        }
    }
    Ok(())
}

fn plate_request_matches(root: &Element, image: &Element) -> bool {
    let Some(wanted) = root.get_attribute("data-want") else {
        return false;
    };
    let Some(wanted_index) = root.get_attribute("data-want-i") else {
        return false;
    };
    let Some(request) = root.get_attribute("data-request") else {
        return false;
    };
    root.is_connected()
        && image.is_connected()
        && Some(wanted) == image.get_attribute("data-src")
        && Some(wanted_index) == image.get_attribute("data-i")
        && Some(request) == image.get_attribute("data-request")
}

fn scene_clip_needs_frame_swap(
    active_index: Option<usize>,
    want_index: usize,
    plate_loaded: bool,
    existing_path: Option<&str>,
    next_path: Option<&str>,
) -> bool {
    active_index == Some(want_index)
        && plate_loaded
        && next_path.is_some()
        && existing_path != next_path
}

fn scene_plate_media_ready(
    image_decoded: bool,
    wanted_video: Option<&str>,
    ready_video: Option<&str>,
) -> bool {
    image_decoded && wanted_video.is_none_or(|wanted| ready_video == Some(wanted))
}

fn activate_plate_request(
    stage: &Element,
    root: &Element,
    image: &Element,
    figures: &Weak<Figures>,
    pending_layout: &Rc<RefCell<Option<crate::presentation::PlateGeometry>>>,
    picture_size: &Rc<Cell<Option<(f64, f64)>>>,
) {
    if !plate_request_matches(root, image) {
        return;
    }
    let request = root.get_attribute("data-request");
    if request.is_some() && root.get_attribute("data-applied-request") == request {
        return;
    }
    let image_decoded = image.get_attribute("data-decoded") == request;
    let frame = image.parent_element();
    if !scene_plate_media_ready(
        image_decoded,
        frame
            .as_ref()
            .and_then(|frame| frame.get_attribute("data-video-want"))
            .as_deref(),
        frame
            .as_ref()
            .and_then(|frame| frame.get_attribute("data-video-ready"))
            .as_deref(),
    ) {
        return;
    }
    let Some(image_index) = image.get_attribute("data-i") else {
        return;
    };
    let Some(key) = root.get_attribute("data-want-key") else {
        return;
    };
    let tick = if root.get_attribute("data-push").as_deref() == Some("a") {
        "b"
    } else {
        "a"
    };
    if root.get_attribute("data-pull").as_deref() != Some("focus") {
        let _set = root.set_attribute("data-push", tick);
    }
    if let Some(request) = request {
        let _set = root.set_attribute("data-applied-request", &request);
    }
    let _set = root.set_attribute("data-transition", &image_index);
    let _set = root.set_attribute("data-active", &image_index);
    let _set = stage.set_attribute("data-plate-key", &key);
    let _set = stage.set_attribute("data-plate-state", "ready");
    let _removed = root.remove_attribute("data-failed");
    if let Some(figures) = figures.upgrade() {
        let layout = pending_layout.borrow().clone();
        let size = image_dimensions(image);
        picture_size.set(size);
        if let Some(style) = layout
            .as_ref()
            .and_then(|layout| plate_head_style(layout, size))
        {
            let _set = root.set_attribute("style", &style);
        } else {
            let _removed = root.remove_attribute("style");
        }
        let _applied = figures.set_plate(layout, size);
    }
    let _set = stage.set_attribute("data-plate-loaded", "");
    let _synced = sync_shot(stage);
}

fn fail_plate_request(
    stage: &Element,
    root: &Element,
    image: &Element,
    figures: &Weak<Figures>,
    pending_layout: &Rc<RefCell<Option<crate::presentation::PlateGeometry>>>,
    picture_size: &Rc<Cell<Option<(f64, f64)>>>,
) {
    if !plate_request_matches(root, image) {
        return;
    }
    let Some(url) = image.get_attribute("data-src") else {
        return;
    };
    let _set = root.set_attribute("data-failed", &url);
    if root.has_attribute("data-active") && stage.has_attribute("data-plate-loaded") {
        let _set = stage.set_attribute("data-plate-state", "ready");
        let active = root.get_attribute("data-active").unwrap_or_default();
        let retained = root
            .query_selector(&format!(".tv-plate-img[data-i=\"{active}\"]"))
            .ok()
            .flatten()
            .and_then(|image| image.get_attribute("data-src"))
            .unwrap_or_default();
        super::presentation_timing::media(
            "plate-kept",
            &stage.get_attribute("data-scene-key").unwrap_or_default(),
            &retained,
            "failed",
            0,
        );
    } else {
        let _set = stage.set_attribute("data-plate-state", "failed");
        let _removed = stage.remove_attribute("data-plate-loaded");
        *pending_layout.borrow_mut() = None;
        let _removed = root.remove_attribute("style");
        picture_size.set(None);
        if let Some(figures) = figures.upgrade() {
            let _applied = figures.set_plate(None, None);
        }
    }
    let _synced = sync_shot(stage);
}

fn image_dimensions(image: &Element) -> Option<(f64, f64)> {
    let width = js_sys::Reflect::get(image.as_ref(), &JsValue::from_str("naturalWidth"))
        .ok()?
        .as_f64()?;
    let height = js_sys::Reflect::get(image.as_ref(), &JsValue::from_str("naturalHeight"))
        .ok()?
        .as_f64()?;
    (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0)
        .then_some((width, height))
}

fn plate_head_style(
    layout: &crate::presentation::PlateGeometry,
    picture_size: Option<(f64, f64)>,
) -> Option<String> {
    let (picture_width, picture_height) = picture_size?;
    let window = web_sys::window()?;
    let width = window.inner_width().ok()?.as_f64()?;
    let height = window.inner_height().ok()?.as_f64()?;
    if !(width > 0.0 && height > 0.0) {
        return None;
    }
    let frame = PlateFrame {
        screen: Rect::new(0.0, 0.0, width, height),
        picture_width,
        picture_height,
    };
    let (x, y) = frame.head_center(layout.head_x, layout.head_y);
    Some(format!(
        "--head-x:{}%;--head-y:{}%",
        x / width * 100.0,
        y / height * 100.0
    ))
}

impl TvStage {
    pub(super) fn mount(document: &Document, parent: &Element) -> Result<Self, JsValue> {
        let root = node(document, parent, "div", "tv-stage", "")?;
        if let Some(splash) = document.query_selector(".tv-splash")? {
            splash.set_attribute("data-done", "")?;
        }
        let art_previous = node(document, &root, "div", "tv-art tv-art-previous", "")?;
        let art = node(document, &root, "div", "tv-art", "")?;
        let loop_layer = node(document, &root, "div", "tv-loop", "")?;
        let home_loop = live_home_video::HomeLoop::new(document, loop_layer)?;
        let atmosphere = node(document, &root, "div", "tv-atmosphere", "")?;
        let mut plate = mount_plate(document, &root)?;
        let scene_graph = live_scene_stage::StageLayer::mount(document, &root)?;
        node(document, &root, "div", "tv-scrim", "")?;
        let figures = Rc::new(Figures::mount(document, &root)?);
        bind_plate_events(&mut plate, &root, Rc::downgrade(&figures))?;
        let safe = node(document, &root, "div", "tv-safe", "")?;
        // The logo and title travel together as one block a themed lobby can place.
        let title_block = node(document, &safe, "div", "tv-title-block", "")?;
        let top = node(document, &title_block, "header", "tv-top", "")?;
        let brand = node(document, &top, "div", "tv-brand", "")?;
        let image = node(document, &brand, "img", "tv-wordmark", "")?;
        image.set_attribute("alt", "DungeonFlux")?;
        image.set_attribute("hidden", "")?;
        let text = node(document, &brand, "span", "tv-wordmark-text", "Dungeon")?;
        node(document, &text, "b", "", "Flux")?;
        let load = {
            let (image, text) = (image.clone(), text.clone());
            Closure::wrap(Box::new(move |_: Event| {
                let _ = image.remove_attribute("hidden");
                let _ = text.set_attribute("hidden", "");
            }) as Box<dyn FnMut(Event)>)
        };
        image.add_event_listener_with_callback("load", load.as_ref().unchecked_ref())?;
        image.set_attribute("src", &format!("{GENERATED}/wordmark"))?;

        let headline = node(document, &title_block, "section", "tv-headline", "")?;
        let title = node(document, &headline, "h1", "tv-title", "")?;
        node(document, &headline, "div", "tv-divider", "")?;
        let place = node(document, &headline, "p", "tv-place", "")?;
        let lede = node(document, &headline, "p", "tv-lede", "")?;

        let lobby_row = node(document, &safe, "section", "tv-lobby-row", "")?;
        let join = node(document, &lobby_row, "aside", "tv-panel tv-join", "")?;
        node(document, &join, "div", "tv-label", "Players join here")?;
        // PLAY-Q01: the code's QR code sits beside the room code on its own light tile.
        let join_row = node(document, &join, "div", "tv-join-row", "")?;
        let qr = node(document, &join_row, "figure", "tv-qr", "")?;
        let qr_image = node(document, &qr, "img", "tv-qr-image", "")?;
        qr_image.set_attribute("src", JOIN_QR)?;
        qr_image.set_attribute("alt", "QR code: scan it to open the join page")?;
        let code_box = node(document, &join_row, "div", "tv-code-box", "")?;
        node(
            document,
            &code_box,
            "div",
            "tv-label tv-code-label",
            "Room code",
        )?;
        let code = node(document, &code_box, "div", "tv-code room-code", "")?;
        let hint = node(document, &join, "p", "tv-hint", "")?;
        let roster = node(document, &lobby_row, "section", "tv-roster", "")?;
        node(document, &roster, "div", "tv-label", "Your party")?;
        let gathered = node(
            document,
            &roster,
            "div",
            "tv-label tv-gathered",
            "The party is gathered",
        )?;
        gathered.set_attribute("hidden", "")?;
        let roster_slot = node(document, &roster, "div", "tv-roster-slot", "")?;

        let stagebox = node(document, &safe, "section", "tv-stagebox", "")?;
        // The Dungeon Master's caption looks unlike a character's speech bubble.
        let caption = node(document, &stagebox, "div", "tv-dm", "")?;
        let caption_head = node(document, &caption, "div", "tv-dm-head", "")?;
        let avatar = node(document, &caption_head, "img", "tv-dm-avatar", "")?;
        set_hidden(&avatar, true)?;
        avatar.set_attribute("alt", "")?;
        node(
            document,
            &caption_head,
            "span",
            "tv-dm-label",
            "Dungeon Master",
        )?;
        let narration = node(document, &caption, "p", "tv-narration", "")?;
        narration.set_attribute("data-voice-role", "narrator")?;
        // The rules wording is read, never spoken, and looks quieter than the story.
        let rules = node(document, &stagebox, "p", "tv-rules", "")?;
        node(document, &rules, "span", "tv-rules-tag", "Rules")?;
        let rules_text = node(document, &rules, "span", "tv-rules-text", "")?;
        node(document, &stagebox, "div", "tv-divider", "")?;
        let speaker = node(document, &stagebox, "div", "tv-speaker", "")?;
        let (_, speaker_glyph, speaker_portrait) = portrait(document, &speaker, "tv-portrait")?;
        let words = node(document, &speaker, "div", "tv-speaker-words", "")?;
        let speaker_name = node(document, &words, "h2", "tv-speaker-name", "")?;
        let speaker_line = node(document, &words, "blockquote", "tv-quote", "")?;

        let party_bar = node(document, &safe, "section", "tv-partybar", "")?;
        let party = node(document, &roster_slot, "div", "tv-party party", "")?;
        let mut cards = Vec::with_capacity(PARTY_SLOTS);
        for _ in 0..PARTY_MAX {
            let card = node(document, &party, "article", "tv-card", "")?;
            let (_, glyph, image) = portrait(document, &card, "tv-portrait")?;
            let words = node(document, &card, "div", "tv-card-words", "")?;
            let name = node(document, &words, "div", "tv-member-name", "")?;
            node(document, &words, "div", "tv-member-role", HERO_ROLE)?;
            let status = node(document, &words, "div", "tv-member-status", "")?;
            cards.push(SeatCard {
                root: card,
                glyph,
                portrait: image,
                name,
                status,
            });
        }
        let caption_lane = node(document, &root, "div", "tv-caption-lane", "")?;
        let lower = mount_lower_third(document, &caption_lane)?;
        let choices = node(document, &caption_lane, "ol", "tv-choices", "")?;
        choices.set_attribute("aria-label", "What the table can do")?;
        let card = mount_card(document, &root)?;
        let sound_gate = mount_sound_gate(document, &root)?;
        let establish = node(document, &root, "div", "tv-establish", "")?;
        establish.set_attribute("aria-hidden", "true")?;
        let establish_title = node(document, &establish, "h2", "tv-establish-title", "")?;
        node(document, &establish, "div", "tv-establish-rule", "")?;
        let establish_place = node(document, &establish, "p", "tv-establish-place", "")?;
        let dock = node(document, &root, "div", "tv-dock", "")?;
        node(document, &root, "div", "tv-peek", "")?;
        Ok(Self {
            root,
            art,
            art_previous,
            plate,
            scene_graph,
            establish_title,
            establish_place,
            lower,
            portraits: RefCell::new(Vec::new()),
            last_narrator_line: Cell::new(0),
            narration_floor: Cell::new(0),
            figures,
            roster,
            gathered,
            drawn: Cell::new(false),
            same_place: Cell::new(false),
            title,
            place,
            lede,
            atmosphere,
            home_loop,
            lobby_row,
            stagebox,
            narration,
            rules,
            rules_text,
            choices,
            card,
            speaker,
            speaker_glyph,
            speaker_portrait,
            speaker_name,
            speaker_line,
            code,
            hint,
            roster_slot,
            party_bar,
            party,
            cards,
            dock,
            wordmark: WordmarkSlot { image, text, load },
            sound_gate,
        })
    }

    /// The stage element, so another screen can mount inside the same persistent shell.
    pub(super) fn root(&self) -> &Element {
        &self.root
    }

    /// The composited characters standing in the scene.
    pub(super) fn figures(&self) -> &Figures {
        &self.figures
    }

    /// Applies the public server graph after the ordinary display fields have been updated.
    pub(super) fn update_scene_graph(
        &self,
        graph: Option<&SceneGraphView>,
        baked_revision: Option<u64>,
    ) -> Result<(), JsValue> {
        self.scene_graph.update(&self.root, graph, baked_revision)?;
        sync_shot(&self.root)
    }

    pub(super) fn update_scene_graph_wire(
        &self,
        graph: Option<&rpc::DisplaySceneGraph>,
        baked_revision: u64,
    ) -> Result<(), JsValue> {
        let graph = graph.and_then(SceneGraphView::from_wire);
        self.update_scene_graph(
            graph.as_ref(),
            (baked_revision != 0).then_some(baked_revision),
        )
    }

    pub(super) fn update_scene_video_wire(&self, state: i32, path: &str) -> Result<(), JsValue> {
        let Some(mut index) = self
            .plate
            .root
            .get_attribute("data-want-i")
            .and_then(|index| index.parse::<usize>().ok())
            .filter(|index| *index < self.plate.images.len())
        else {
            for video_loop in self.plate.video_loops.borrow().iter() {
                video_loop.release();
            }
            self.plate.video_paths.borrow_mut().fill(None);
            for frame in &self.plate.frames {
                let _removed = frame.remove_attribute("data-video-want");
                let _removed = frame.remove_attribute("data-video-ready");
            }
            return Ok(());
        };
        let path = (state == rpc::HomeVideoState::Ready as i32)
            .then(|| scene_video_url(path))
            .flatten()
            .map(str::to_owned);
        let active_index = self
            .plate
            .root
            .get_attribute("data-active")
            .and_then(|active| active.parse::<usize>().ok());
        let transitioning_index = self
            .plate
            .root
            .get_attribute("data-transition")
            .and_then(|transitioning| transitioning.parse::<usize>().ok());
        if self.root.get_attribute("data-tv-mode").as_deref() != Some("scene") {
            for video_loop in self.plate.video_loops.borrow().iter() {
                video_loop.release();
            }
            self.plate.video_paths.borrow_mut().fill(None);
            for frame in &self.plate.frames {
                let _removed = frame.remove_attribute("data-video-want");
                let _removed = frame.remove_attribute("data-video-ready");
            }
        } else if state == rpc::HomeVideoState::Pending as i32 && active_index == Some(index) {
            // Keep a playing clip over its plate while the server prepares its replacement.
        } else {
            let needs_swap = {
                let video_paths = self.plate.video_paths.borrow();
                scene_clip_needs_frame_swap(
                    active_index,
                    index,
                    self.root.has_attribute("data-plate-loaded"),
                    video_paths.get(index).and_then(Option::as_deref),
                    path.as_deref(),
                )
            };
            if needs_swap {
                let target = 1 - index;
                let Some(url) = self.plate.root.get_attribute("data-want") else {
                    return Ok(());
                };
                let Some(key) = self.root.get_attribute("data-plate-key") else {
                    return Ok(());
                };
                let Some(image) = self.plate.images.get(target) else {
                    return Ok(());
                };
                let request = self
                    .plate
                    .request
                    .get()
                    .checked_add(1)
                    .ok_or_else(|| JsValue::from_str("plate request generation exhausted"))?;
                self.plate.request.set(request);
                self.plate
                    .root
                    .set_attribute("data-request", &request.to_string())?;
                self.plate
                    .root
                    .set_attribute("data-want-i", &target.to_string())?;
                self.plate.root.set_attribute("data-want-key", &key)?;
                self.plate.root.set_attribute("data-pull", "focus")?;
                image.set_attribute("data-request", &request.to_string())?;
                image.set_attribute("data-src", &url)?;
                image.remove_attribute("src")?;
                image.set_attribute("src", &url)?;
                self.plate.next.set(index);
                index = target;
            }
            let Some(frame) = self.plate.frames.get(index) else {
                return Ok(());
            };
            let video_changed = self
                .plate
                .video_paths
                .borrow()
                .get(index)
                .and_then(Option::as_deref)
                != path.as_deref();
            if video_changed {
                if let Some(path) = path.as_deref() {
                    super::presentation_timing::media(
                        "clip-requested",
                        &self
                            .root
                            .get_attribute("data-scene-key")
                            .unwrap_or_default(),
                        path,
                        "replaced",
                        0,
                    );
                }
                if let Some(video) = frame.query_selector("video.tv-loop-clip")? {
                    video.remove_attribute("data-timing-playing")?;
                }
                frame.remove_attribute("data-video-ready")?;
                if let Some(path) = path.as_deref() {
                    frame.set_attribute("data-video-want", path)?;
                } else {
                    frame.remove_attribute("data-video-want")?;
                }
            }
            self.plate.video_paths.borrow_mut()[index] = path.clone();
            if transitioning_index != Some(index)
                && let Some(video_loop) = self.plate.video_loops.borrow().get(index)
            {
                // The inactive frame is ready before the plate cross-fade reaches it.
                video_loop.apply(path.as_deref())?;
            }
            if video_changed
                && path.is_none()
                && active_index != Some(index)
                && let Some(image) = self.plate.images.get(index)
            {
                activate_plate_request(
                    &self.root,
                    &self.plate.root,
                    image,
                    &Rc::downgrade(&self.figures),
                    &self.plate.pending_layout,
                    &self.plate.picture_size,
                );
            }
        }
        Ok(())
    }

    /// Shows `style` as the painting. A different painting leaves the previous one to fade
    /// out underneath while the new one fades in; the same one changes nothing.
    pub(super) fn set_art(&self, style: &str) -> Result<(), JsValue> {
        let before = self.art.get_attribute("style");
        if before.as_deref() == Some(style) {
            return Ok(());
        }
        if let Some(before) = before {
            self.art_previous.set_attribute("style", &before)?;
            let tick = if self.root.get_attribute("data-art-swap").as_deref() == Some("a") {
                "b"
            } else {
                "a"
            };
            self.root.set_attribute("data-art-swap", tick)?;
        }
        self.art.set_attribute("style", style)
    }

    /// Where the director is in a scene change or beat; the stylesheet shows each state.
    pub(super) fn set_stage_state(&self, state: &str) -> Result<(), JsValue> {
        if state.is_empty() {
            self.root.remove_attribute("data-stage")
        } else {
            self.root.set_attribute("data-stage", state)
        }
    }

    /// Fills the title or closing card; the stylesheet shows it while the director is in the
    /// matching state.
    pub(super) fn show_card(&self, document: &Document, card: &Card) -> Result<(), JsValue> {
        let shown = &self.card;
        shown.root.set_attribute(
            "data-kind",
            match card.kind {
                CardKind::Title => "title",
                CardKind::Victory => "victory",
                CardKind::Defeat => "defeat",
                CardKind::End => "end",
            },
        )?;
        shown.kicker.set_text_content(Some(&card.kicker));
        shown.headline.set_text_content(Some(&card.headline));
        shown.line.set_text_content(Some(&card.subline));
        shown.place.set_text_content(Some(&card.place));
        set_hidden(&shown.place, card.place.is_empty())?;
        set_hidden(&shown.line, card.subline.is_empty())?;
        if card.kind != CardKind::End {
            set_hidden(&shown.rewards, true)?;
        }
        shown.heroes.set_text_content(None);
        set_hidden(&shown.heroes, card.heroes.is_empty())?;
        for hero in &card.heroes {
            let item = node(document, &shown.heroes, "li", "tv-bigcard-hero", "")?;
            let crest = node(document, &item, "span", "tv-bigcard-crest", "")?;
            crest.set_attribute("style", &format!("background-image:url({})", hero.portrait))?;
            node(document, &item, "b", "tv-bigcard-name", &hero.name)?;
            node(document, &item, "span", "tv-bigcard-role", &hero.role)?;
        }
        Ok(())
    }

    /// Which speaker the stage's words belong to; the stylesheet shows the Dungeon Master's
    /// caption or the character's bubble accordingly, never both.
    pub(super) fn set_speaker(&self, speaker: &str) -> Result<(), JsValue> {
        if speaker.is_empty() {
            self.root.remove_attribute("data-speaker")
        } else {
            self.root.set_attribute("data-speaker", speaker)
        }
    }

    /// Applies one transition of the narrator line's machine to the Dungeon Master's box: the
    /// text is set as the line opens, never earlier. Returns whether the line opened.
    pub(super) fn narrator_line(
        &self,
        id: u64,
        state: LineState,
        text: &str,
    ) -> Result<bool, JsValue> {
        self.last_narrator_line
            .set(self.last_narrator_line.get().max(id));
        // PLAY-V12: the plate may be cut to once the narrator of this scene has spoken; the words
        // of the scene before, closing while this one is drawn, do not count.
        if id > self.narration_floor.get()
            && matches!(
                state,
                LineState::Lingering | LineState::Closing | LineState::Done
            )
        {
            self.narration_over()?;
        }
        let opened = matches!(
            state,
            LineState::Opening | LineState::Speaking | LineState::Lingering
        ) && self.narration.text_content().as_deref() != Some(text);
        if opened {
            self.narration.set_text_content(Some(text));
            self.narration.set_attribute("title", text)?;
        }
        if let Some(caption) = self.narration.parent_element() {
            caption.set_attribute("data-line-id", &id.to_string())?;
            caption.set_attribute("data-line-state", state.name())?;
        }
        // VICTORY-001: a finished line stays on screen until the next scene's
        // entry clears it (see `clear_narration` on `SceneState::Entering`), so
        // the narrator box never sits empty while the table waits.
        if state == LineState::Done {
            self.narration.remove_attribute("title")?;
        }
        match state {
            LineState::Speaking => self.narration.class_list().add_1("speaking")?,
            LineState::Lingering | LineState::Closing | LineState::Done => {
                self.narration.class_list().remove_1("speaking")?;
            }
            _ => {}
        }
        super::presentation_timing::line(
            id,
            state,
            None,
            text.chars().count() as u32,
            crate::stage_machines::reading_time_ms(text),
            &self
                .root
                .get_attribute("data-scene-key")
                .unwrap_or_default(),
        );
        Ok(opened)
    }

    /// Empties the Dungeon Master's box: a new scene starts with none of the old scene's words.
    pub(super) fn clear_narration(&self) -> Result<(), JsValue> {
        if let Some(id) = self
            .narration
            .parent_element()
            .and_then(|caption| caption.get_attribute("data-line-id"))
            .and_then(|id| id.parse::<u64>().ok())
        {
            super::presentation_timing::close_reason(id, "scene-change");
            super::presentation_timing::line(
                id,
                LineState::Done,
                None,
                0,
                0,
                &self
                    .root
                    .get_attribute("data-scene-key")
                    .unwrap_or_default(),
            );
        }
        self.narration.set_text_content(None);
        self.narration.remove_attribute("title")?;
        self.narration.class_list().remove_1("speaking")?;
        if let Some(caption) = self.narration.parent_element() {
            caption.remove_attribute("data-line-id")?;
            caption.remove_attribute("data-line-state")?;
        }
        Ok(())
    }

    /// Restarts the staggered title, narration and figure entrance.
    pub(super) fn begin_entering(&self) -> Result<(), JsValue> {
        // A beat in the same place only cross-fades its title; a new place replays the entrance.
        self.root.set_attribute(
            "data-title-anim",
            if self.same_place.get() {
                "fade"
            } else {
                "full"
            },
        )?;
        let tick = if self.root.get_attribute("data-scene-swap").as_deref() == Some("a") {
            "b"
        } else {
            "a"
        };
        self.root.set_attribute("data-scene-swap", tick)
    }

    /// The container for recovery controls; it is emptied and refilled on every render.
    /// Silent context on a late join sits apart from the single active subtitle lane.
    pub(super) fn set_scene_context(&self, text: &str) -> Result<(), JsValue> {
        if text.trim().is_empty() {
            return Ok(());
        }
        let context = match self.root.query_selector(".tv-scene-context")? {
            Some(context) => context,
            None => {
                let document = self
                    .root
                    .owner_document()
                    .ok_or_else(|| JsValue::from_str("display document absent"))?;
                let context = node(&document, &self.root, "aside", "tv-scene-context", "")?;
                context.set_attribute("aria-label", "Scene context")?;
                context
            }
        };
        context.set_text_content(Some(text));
        Ok(())
    }

    pub(super) fn dock(&self) -> &Element {
        &self.dock
    }

    pub(super) fn update(&self, model: &TvModel) -> Result<(), JsValue> {
        let lobby = model.mode == TvMode::Lobby;
        if self.root.get_attribute("data-scene-key").as_deref() != Some(&model.scene_key)
            && let Some(context) = self.root.query_selector(".tv-scene-context")?
        {
            context.remove();
        }
        self.root
            .set_attribute("data-scene-key", &model.scene_key)?;
        self.root
            .set_attribute("data-tv-mode", if lobby { "lobby" } else { "scene" })?;
        self.root
            .set_attribute("data-art-source", model.art_source.label())?;
        self.root
            .set_attribute("data-visual-scene-id", &model.visual_scene_id)?;
        self.set_art(&image_layers(&model.art, model.art_fallback))?;
        live_home::apply(&self.root, &self.atmosphere, model.home.as_ref())?;
        // The loop plays only in the lobby; a story scene or a fight never shows it.
        self.home_loop.apply(if lobby {
            model.home_video.as_deref()
        } else {
            None
        })?;
        let known_place = self.place.text_content().unwrap_or_default();
        self.same_place
            .set(!model.place.is_empty() && known_place == model.place);
        let lobby_title = if lobby && !model.adventure_name.is_empty() {
            &model.adventure_name
        } else {
            &model.title
        };
        self.title.set_text_content(Some(lobby_title));
        self.place.set_text_content(Some(&model.place));
        set_hidden(&self.place, model.place.is_empty())?;
        self.lede.set_text_content(Some(&model.lede));
        self.rules_text.set_text_content(Some(&model.rules_note));
        set_hidden(&self.rules, model.rules_note.is_empty() || lobby)?;
        self.set_choices(
            if lobby || model.end.is_some() {
                &[]
            } else {
                &model.choices
            },
            model.choice_hints_hidden,
        )?;
        match &model.end {
            Some(card) => {
                self.root.set_attribute("data-ended", "")?;
                let document = self
                    .root
                    .owner_document()
                    .ok_or_else(|| JsValue::from_str("display document absent"))?;
                self.show_card(&document, card)?;
            }
            None => {
                self.root.remove_attribute("data-ended")?;
                self.root.remove_attribute("data-ending-ready")?;
            }
        }
        set_hidden(&self.lobby_row, !lobby)?;
        set_hidden(
            &self.stagebox,
            lobby || model.end.is_some() || model.check.is_some(),
        )?;
        // The party cards are one set of nodes that moves with the mode.
        let home = if lobby {
            &self.roster_slot
        } else {
            &self.party_bar
        };
        let moved = !self
            .party
            .parent_node()
            .is_some_and(|parent| parent.is_same_node(Some(home.as_ref())));
        if moved {
            home.append_child(&self.party)?;
        }
        self.code.set_text_content(Some(&model.room_code));
        // The code's length sizes it to the join card (live_home.css).
        self.code.set_attribute(
            "style",
            &format!("--chars:{}", model.room_code.chars().count().max(1)),
        )?;
        // A code that leads to this computer's own address is not offered for scanning
        // (live_join.css hides the QR tile).
        let local_only = is_loopback_join_host(&model.join_host);
        if let Some(join) = self.hint.parent_element() {
            if local_only {
                join.set_attribute("data-local-only", "")?;
            } else {
                join.remove_attribute("data-local-only")?;
            }
        }
        if local_only {
            self.hint.set_text_content(Some(
                "Phones cannot join yet: the game was started for this computer only",
            ));
        } else {
            self.hint
                .set_text_content(Some(&format!("Or enter the code at {}", model.join_host)));
        }
        match &model.speaker {
            Some(speaker) if !lobby => {
                set_hidden(&self.speaker, false)?;
                self.speaker_name.set_text_content(Some(&speaker.name));
                self.speaker_line.set_text_content(Some(&speaker.line));
                self.speaker_glyph
                    .set_text_content(Some(&initial(&speaker.name)));
                self.speaker_portrait.set_attribute(
                    "style",
                    &speaker
                        .portrait
                        .as_deref()
                        .map(|url| format!("background-image:url({url})"))
                        .unwrap_or_default(),
                )?;
            }
            _ => set_hidden(&self.speaker, true)?,
        }
        let announce = self.drawn.get() && lobby;
        // The lobby shows four seats at least, with open seats for the empty ones, up to six in one
        // row; a scene keeps its two cards.
        let shown = if lobby {
            model.party.len().clamp(LOBBY_SEATS_MIN, PARTY_MAX)
        } else {
            PARTY_SLOTS
        };
        self.roster
            .set_attribute("data-count", &shown.to_string())?;
        let mut newly_ready = false;
        for (index, card) in self.cards.iter().enumerate() {
            set_hidden(&card.root, index >= shown)?;
            let before = card.root.get_attribute("data-state");
            let state = match model.party.get(index) {
                None => "empty",
                Some(member) if member.ready => "ready",
                Some(_) => "creating",
            };
            card.root.set_attribute("data-state", state)?;
            // The one-shot reveal belongs to the lobby, not to a card that moves to a new place.
            if moved || !lobby {
                card.root.remove_attribute("data-reveal")?;
            } else if announce && before.as_deref() == Some("creating") && state == "ready" {
                card.root.set_attribute("data-reveal", "")?;
                newly_ready = true;
            }
            match model.party.get(index) {
                Some(member) => {
                    card.root.remove_attribute("data-empty")?;
                    card.root
                        .set_attribute("data-ready", if member.ready { "true" } else { "false" })?;
                    // The server withholds a name until the hero exists.
                    card.name.set_text_content(Some(if member.ready {
                        &member.name
                    } else {
                        "A hero is being forged"
                    }));
                    card.status.set_text_content(Some(&member.status));
                    card.glyph.set_text_content(Some(&initial(if member.ready {
                        &member.name
                    } else {
                        "?"
                    })));
                    card.portrait.set_attribute(
                        "style",
                        &if member.ready {
                            format!(
                                "background-image:url({GENERATED}/figure-hero-{}-documentary-photo)",
                                index + 1
                            )
                        } else {
                            String::new()
                        },
                    )?;
                }
                None => {
                    card.root.set_attribute("data-empty", "")?;
                    card.root.set_attribute("data-ready", "false")?;
                    card.name.set_text_content(Some("Open seat"));
                    card.status.set_text_content(Some("Waiting to join"));
                    card.glyph.set_text_content(Some("+"));
                    card.portrait.set_attribute("style", "")?;
                }
            }
        }
        let seats = model.party.len();
        let ready_seats = model.party.iter().filter(|member| member.ready).count();
        let gathered = lobby && seats >= PARTY_SLOTS && ready_seats == seats;
        let was_gathered = self.roster.has_attribute("data-gathered");
        if gathered {
            self.roster.set_attribute("data-gathered", "")?;
        } else {
            self.roster.remove_attribute("data-gathered")?;
        }
        set_hidden(&self.gathered, !lobby)?;
        if lobby {
            let ready_heroes = if ready_seats == 1 { "hero" } else { "heroes" };
            let next_step = if gathered {
                "Any player can begin the adventure from their phone"
            } else {
                "Waiting for heroes"
            };
            self.gathered.set_text_content(Some(&format!(
                "{ready_seats} {ready_heroes} ready. {next_step}."
            )));
        }
        if newly_ready {
            live_sfx::play(Cue::HeroReady, Duration::ZERO);
        }
        if announce && gathered && !was_gathered {
            live_sfx::play(Cue::PartyGathered, GATHERED_SOUND_DELAY);
        }
        let establish_title = if !self.drawn.get() && !model.adventure_name.is_empty() {
            &model.adventure_name
        } else {
            &model.title
        };
        self.establish_title.set_text_content(Some(establish_title));
        self.establish_place.set_text_content(Some(&model.place));
        set_hidden(&self.establish_place, model.place.is_empty())?;
        *self.portraits.borrow_mut() = model
            .cast
            .iter()
            .map(|actor| (actor.name.clone(), actor.portrait.clone()))
            .collect();
        self.set_plate(&model.plate, &model.scene_key, model.narrated)?;
        if lobby {
            self.figures.update(
                &[],
                &model.scene_key,
                &model.art,
                model.family,
                StageMode::Scene,
            )?;
        } else {
            self.figures.update(
                &model.cast,
                &model.scene_key,
                &model.art,
                model.family,
                StageMode::Scene,
            )?;
        }
        self.figures.set_check(model.check.as_ref())?;
        if model.check.is_some() {
            self.root.set_attribute("data-check-active", "")?;
        } else {
            self.root.remove_attribute("data-check-active")?;
        }
        self.drawn.set(true);
        refresh_timing_visibility(&self.root);
        Ok(())
    }

    pub(super) fn set_end_rewards(&self, rewards: &[String]) -> Result<(), JsValue> {
        self.card.rewards.set_text_content(None);
        set_hidden(&self.card.rewards, rewards.is_empty())?;
        let document = self
            .root
            .owner_document()
            .ok_or_else(|| JsValue::from_str("display document absent"))?;
        for reward in rewards {
            node(
                &document,
                &self.card.rewards,
                "p",
                "tv-bigcard-reward",
                reward,
            )?;
        }
        Ok(())
    }

    pub(super) fn set_end_card_ready(&self, ready: bool) -> Result<(), JsValue> {
        if ready {
            self.root.set_attribute("data-ending-ready", "")
        } else {
            self.root.remove_attribute("data-ending-ready")
        }
    }

    /// The read-only row of numbered chips: what the table can do now. It is replaced only
    /// when the labels change, so a redraw never restarts its animation.
    pub(super) fn set_choices(&self, labels: &[String], hidden: bool) -> Result<(), JsValue> {
        let shown: Vec<&str> = labels.iter().map(String::as_str).collect();
        let signature = shown.join("\u{1f}");
        // `data-chips="off"` is the table's setting that hides the hints (the stylesheet then
        // gives their room to the narrator box); otherwise the count of chips is recorded.
        let chips = if hidden {
            "off".to_owned()
        } else {
            shown.len().to_string()
        };
        self.root.set_attribute("data-chips", &chips)?;
        if self.choices.get_attribute("data-labels").as_deref() == Some(signature.as_str()) {
            return Ok(());
        }
        self.choices.set_attribute("data-labels", &signature)?;
        self.choices.set_text_content(None);
        let document = self.root.owner_document().ok_or("document absent")?;
        for (number, label) in shown.iter().enumerate() {
            let chip = node(&document, &self.choices, "li", "tv-choice", "")?;
            node(
                &document,
                &chip,
                "span",
                "tv-choice-number",
                &(number + 1).to_string(),
            )?;
            node(&document, &chip, "span", "tv-choice-label", label)?;
        }
        Ok(())
    }

    /// PLAY-V09: starts fetching a plate picture the moment a view names it, usually while the
    /// scene before it is still being spoken, so that the scene change can show it at once.
    pub(super) fn preload_plate(&self, url: &str) -> Result<(), JsValue> {
        let mut preloads = self.plate.preloads.borrow_mut();
        if preloads.iter().any(|(known, _)| known == url) {
            return Ok(());
        }
        let document = self.root.owner_document().ok_or("document absent")?;
        let image = document.create_element("img")?;
        image.set_attribute("src", url)?;
        preloads.push((url.to_owned(), image));
        if preloads.len() > MAXIMUM_PRELOADS {
            preloads.remove(0);
        }
        Ok(())
    }

    /// PLAY-V12: the narration of the scene's entry is over (or the scene has none): a ready plate
    /// may be cut to now. It stays over for the rest of the scene, so the narration of a look's
    /// result never puts the establishing shot back.
    pub(super) fn narration_over(&self) -> Result<(), JsValue> {
        self.root.set_attribute("data-narration-over", "")?;
        sync_shot(&self.root)
    }

    /// PLAY-V12: a new scene opens on its establishing shot: the place picture alone, pushed in
    /// slowly. Its entry narration is over at once only when it has none, or when the page has just
    /// come up on it (a reload tells nothing again).
    fn begin_entry(&self, narration_over: bool) -> Result<(), JsValue> {
        self.narration_floor.set(self.last_narrator_line.get());
        if narration_over {
            self.root.set_attribute("data-narration-over", "")?;
        } else {
            self.root.remove_attribute("data-narration-over")?;
        }
        let tick = if self.root.get_attribute("data-shot-tick").as_deref() == Some("a") {
            "b"
        } else {
            "a"
        };
        self.root.set_attribute("data-shot-tick", tick)
    }

    /// PLAY-V12: shows what a character says in the lower third, with the speaker's portrait and
    /// name, so the establishing shot always says who is talking. The stylesheet draws it only
    /// over an establishing shot and only while the line is the one being heard.
    pub(super) fn lower_third(
        &self,
        speaker: &str,
        text: &str,
        state: LineState,
    ) -> Result<(), JsValue> {
        let lower = &self.lower;
        if lower.text.text_content().as_deref() != Some(text)
            || lower.name.text_content().as_deref() != Some(speaker)
        {
            lower.name.set_text_content(Some(speaker));
            lower.text.set_text_content(Some(text));
            lower.glyph.set_text_content(Some(&initial(speaker)));
            let portrait = self
                .portraits
                .borrow()
                .iter()
                .find(|(name, _)| name == speaker)
                .and_then(|(_, path)| asset_url(path));
            lower.portrait.set_attribute(
                "style",
                &portrait
                    .map(|url| format!("background-image:url({url})"))
                    .unwrap_or_default(),
            )?;
        }
        lower.root.set_attribute("data-line-state", state.name())
    }

    /// PLAY-V09: shows the scene plate the server made, or leaves the scene as it was.
    ///
    /// `data-plate-state` is the server's word (none, pending, ready, failed; failed also when the
    /// picture does not load) and `data-plate-key` names the plate. `data-plate-loaded` says the
    /// picture has loaded; `data-shot` (see [`sync_shot`]) says which picture the scene shows: its
    /// cut-outs are never drawn while a plate is pending or ready, and a ready plate is cut to
    /// only once the narration of the scene's entry is over. A new picture for the scene already
    /// on screen (a mood edit) comes in as a focus pull.
    fn set_plate(&self, plate: &PlateView, scene_key: &str, narrated: bool) -> Result<(), JsValue> {
        if self.plate.seen.borrow().as_str() != scene_key {
            self.begin_entry(!narrated || !self.drawn.get())?;
        }
        let shown = self.apply_plate(plate, scene_key);
        sync_shot(&self.root)?;
        shown
    }

    fn apply_plate(&self, plate: &PlateView, scene_key: &str) -> Result<(), JsValue> {
        let layer = &self.plate;
        let failed = layer.root.get_attribute("data-failed");
        let usable = match (&plate.state, &plate.url) {
            (PlateState::Ready, Some(url)) if failed.as_deref() != Some(url.as_str()) => Some(url),
            _ => None,
        };
        let state = match (plate.state, usable) {
            (PlateState::Ready, None) => PlateState::Failed,
            (state, _) => state,
        };
        let Some(url) = usable else {
            if matches!(state, PlateState::Pending | PlateState::Failed)
                && layer.root.has_attribute("data-active")
                && self.root.has_attribute("data-plate-loaded")
            {
                self.root.set_attribute("data-plate-state", "ready")?;
                *layer.seen.borrow_mut() = scene_key.to_owned();
                return Ok(());
            }
            self.root.set_attribute("data-plate-state", state.name())?;
            self.root.remove_attribute("data-plate-key")?;
            self.root.remove_attribute("data-plate-loaded")?;
            layer.root.remove_attribute("data-want")?;
            layer.root.remove_attribute("data-push")?;
            layer.root.remove_attribute("data-pull")?;
            layer.root.remove_attribute("data-want-i")?;
            layer.root.remove_attribute("data-request")?;
            layer.root.remove_attribute("data-want-key")?;
            for video_loop in layer.video_loops.borrow().iter() {
                video_loop.release();
            }
            layer.video_paths.borrow_mut().fill(None);
            for frame in &layer.frames {
                let _removed = frame.remove_attribute("data-video-want");
                let _removed = frame.remove_attribute("data-video-ready");
            }
            layer.scene.borrow_mut().clear();
            *layer.seen.borrow_mut() = scene_key.to_owned();
            *layer.pending_layout.borrow_mut() = None;
            self.figures.set_plate(None, None)?;
            return Ok(());
        };
        self.root.set_attribute("data-plate-state", "ready")?;
        if layer.root.get_attribute("data-want").as_deref() == Some(url.as_str()) {
            // The same picture can arrive with a new layout: its speaker has moved.
            let layout = plate.layout.clone();
            *layer.pending_layout.borrow_mut() = layout.clone();
            if self.root.has_attribute("data-plate-loaded")
                && layer.root.get_attribute("data-active")
                    == layer.root.get_attribute("data-want-i")
            {
                if let Some(style) = layout
                    .as_ref()
                    .and_then(|layout| plate_head_style(layout, layer.picture_size.get()))
                {
                    layer.root.set_attribute("style", &style)?;
                } else {
                    layer.root.remove_attribute("style")?;
                }
                self.figures.set_plate(layout, layer.picture_size.get())?;
            }
            return Ok(());
        }
        let same_scene =
            layer.scene.borrow().as_str() == scene_key && layer.root.has_attribute("data-want");
        *layer.pending_layout.borrow_mut() = plate.layout.clone();
        let active = layer
            .root
            .get_attribute("data-active")
            .and_then(|active| active.parse::<usize>().ok())
            .filter(|index| *index < layer.images.len())
            .filter(|_| self.root.has_attribute("data-plate-loaded"));
        let next = active.map_or_else(|| layer.next.get(), |active| 1 - active);
        let Some(image) = layer.images.get(next) else {
            return Ok(());
        };
        if let Some(video_loop) = layer.video_loops.borrow().get(next) {
            video_loop.release();
        }
        if let Some(frame) = layer.frames.get(next) {
            frame.remove_attribute("data-video-want")?;
            frame.remove_attribute("data-video-ready")?;
        }
        layer.video_paths.borrow_mut()[next] = None;
        let request = layer
            .request
            .get()
            .checked_add(1)
            .ok_or_else(|| JsValue::from_str("plate request generation exhausted"))?;
        layer.request.set(request);
        layer
            .root
            .set_attribute("data-request", &request.to_string())?;
        image.set_attribute("data-request", &request.to_string())?;
        layer.root.set_attribute("data-want-i", &next.to_string())?;
        layer.next.set(1 - next);
        let key = url
            .rsplit('/')
            .next()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| JsValue::from_str("plate URL has no asset key"))?;
        layer.root.set_attribute("data-want-key", key)?;
        layer.root.set_attribute("data-want", url)?;
        if same_scene {
            // An edit of the plate on screen: the old one stays until the new one has loaded.
            layer.root.set_attribute("data-pull", "focus")?;
        } else {
            layer.root.remove_attribute("data-pull")?;
            *layer.scene.borrow_mut() = scene_key.to_owned();
        }
        *layer.seen.borrow_mut() = scene_key.to_owned();
        image.set_attribute("data-src", url)?;
        image.set_attribute("src", url)?;
        super::presentation_timing::media(
            "plate-requested",
            scene_key,
            url,
            if same_scene {
                "replaced"
            } else {
                "scene-change"
            },
            1200,
        );
        Ok(())
    }

    pub(super) fn dispose(self) -> Result<(), JsValue> {
        self.sound_gate.layer.remove();
        for (image, (load, error)) in self.plate.images.iter().zip(&self.plate.listeners) {
            // Listeners die with their elements; removing them is hygiene.
            let _removed =
                image.remove_event_listener_with_callback("load", load.as_ref().unchecked_ref());
            let _removed =
                image.remove_event_listener_with_callback("error", error.as_ref().unchecked_ref());
        }
        for (frame, listener) in self.plate.frames.iter().zip(&self.plate.frame_listeners) {
            let _removed = frame.remove_event_listener_with_callback(
                "transitionend",
                listener.as_ref().unchecked_ref(),
            );
        }
        for (video, event_name, listener) in &self.plate.video_ready_listeners {
            let _removed = video.remove_event_listener_with_callback_and_bool(
                event_name,
                listener.as_ref().unchecked_ref(),
                *event_name != "canplay",
            );
        }
        for (button, listener) in [self.sound_gate.start.clone(), self.sound_gate.skip.clone()]
            .iter()
            .zip(&self.sound_gate.dismiss_listeners)
        {
            let _removed = button
                .remove_event_listener_with_callback("click", listener.as_ref().unchecked_ref());
        }
        for listener in &self.sound_gate.skip_guards {
            let _removed = self.sound_gate.skip.remove_event_listener_with_callback(
                "pointerdown",
                listener.as_ref().unchecked_ref(),
            );
            let _removed = self
                .sound_gate
                .skip
                .remove_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref());
        }
        for video_loop in self.plate.video_loops.borrow().iter() {
            video_loop.release();
        }
        self.figures.dispose();
        let removed = self.wordmark.image.remove_event_listener_with_callback(
            "load",
            self.wordmark.load.as_ref().unchecked_ref(),
        );
        self.wordmark.text.remove_attribute("hidden")?;
        self.home_loop.release();
        self.root.remove();
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::super::rpc;
    use super::{
        PlateState, PlateView, TvStage, check_display, first_art_asset, is_loopback_join_host,
        plate_asset_key, scene_clip_needs_frame_swap, scene_plate_media_ready, scene_video_url,
    };
    fn mounted_stage() -> (web_sys::Element, TvStage) {
        let document = web_sys::window().unwrap().document().unwrap();
        let parent = document.create_element("section").unwrap();
        document.body().unwrap().append_child(&parent).unwrap();
        let stage = TvStage::mount(&document, &parent).unwrap();
        (parent, stage)
    }

    fn seed_active_plate(stage: &TvStage) {
        let image = &stage.plate.images[0];
        image.set_attribute("data-i", "0").unwrap();
        image
            .set_attribute("data-src", "/gameplay/imagery/old")
            .unwrap();
        stage.plate.root.set_attribute("data-active", "0").unwrap();
        stage
            .plate
            .root
            .set_attribute("data-want", "/gameplay/imagery/old")
            .unwrap();
        stage.plate.root.set_attribute("data-want-i", "0").unwrap();
        stage.plate.next.set(1);
        stage.root.set_attribute("data-tv-mode", "scene").unwrap();
        stage
            .root
            .set_attribute("data-plate-state", "ready")
            .unwrap();
        stage.root.set_attribute("data-plate-loaded", "").unwrap();
        stage.root.set_attribute("data-shot", "plate").unwrap();
        *stage.plate.scene.borrow_mut() = "old-scene".to_owned();
        *stage.plate.seen.borrow_mut() = "old-scene".to_owned();
    }

    fn finish_stage(parent: web_sys::Element, stage: TvStage) {
        stage.dispose().unwrap();
        parent.remove();
    }

    #[test]
    fn visual_source_hashes_match_the_selected_art_or_plate_key() {
        let style = "background-image:url(/assets/photo.webp),url(/assets/fallback.webp)";
        assert_eq!(first_art_asset(style), Some("/assets/photo.webp"));
        assert_eq!(first_art_asset("none"), None);

        let key = "a".repeat(64);
        assert_eq!(plate_asset_key(&format!("/gameplay/imagery/{key}")), key);
        assert_eq!(plate_asset_key(&key), key);
    }

    #[test]
    fn scene_clip_paths_require_the_server_video_route_and_key() {
        let path = format!("/gameplay/home-video/{}", "a".repeat(64));
        assert_eq!(scene_video_url(&path), Some(path.as_str()));
        assert_eq!(scene_video_url("https://example.test/clip.mp4"), None);
        assert_eq!(scene_video_url("/gameplay/home-video/not-a-key"), None);
    }

    #[test]
    fn changed_scene_clip_uses_the_inactive_plate_frame_until_it_is_ready() {
        assert!(scene_clip_needs_frame_swap(
            Some(0),
            0,
            true,
            Some("/gameplay/home-video/old"),
            Some("/gameplay/home-video/new"),
        ));
        assert!(!scene_clip_needs_frame_swap(
            Some(0),
            0,
            true,
            Some("/gameplay/home-video/new"),
            Some("/gameplay/home-video/new"),
        ));
        assert!(!scene_clip_needs_frame_swap(
            Some(0),
            0,
            false,
            None,
            Some("/gameplay/home-video/new"),
        ));
    }

    #[test]
    fn scene_plate_waits_for_both_the_image_and_requested_clip() {
        let clip = "/gameplay/home-video/new";
        assert!(!scene_plate_media_ready(false, Some(clip), Some(clip)));
        assert!(!scene_plate_media_ready(true, Some(clip), None));
        assert!(!scene_plate_media_ready(
            true,
            Some(clip),
            Some("/gameplay/home-video/old")
        ));
        assert!(scene_plate_media_ready(true, Some(clip), Some(clip)));
        assert!(scene_plate_media_ready(true, None, None));
    }

    #[test]
    fn loopback_join_hosts_are_not_shown_as_phone_addresses() {
        assert!(is_loopback_join_host("127.0.0.1:63319"));
        assert!(is_loopback_join_host("127.42.0.9:63319"));
        assert!(is_loopback_join_host("[::1]:63319"));
        assert!(is_loopback_join_host("localhost:63319"));
    }

    #[test]
    fn network_join_hosts_remain_available_to_phones() {
        assert!(!is_loopback_join_host("192.168.1.20:63319"));
        assert!(!is_loopback_join_host("game.example.com:63319"));
    }

    #[test]
    fn narration_in_a_new_scene_does_not_hide_the_plate_awaiting_its_successor() {
        let (parent, stage) = mounted_stage();
        seed_active_plate(&stage);
        let plate = PlateView {
            state: PlateState::Ready,
            url: Some("/gameplay/imagery/next-plate".to_owned()),
            layout: None,
        };
        stage.set_plate(&plate, "next-scene", true).unwrap();
        assert_eq!(
            stage.root.get_attribute("data-shot").as_deref(),
            Some("plate")
        );
        assert_eq!(
            stage.plate.root.get_attribute("data-active").as_deref(),
            Some("0")
        );
        finish_stage(parent, stage);
    }

    #[test]
    fn repeated_media_ready_callback_does_not_restart_the_crossfade() {
        let (parent, stage) = mounted_stage();
        seed_active_plate(&stage);
        let url = "/gameplay/imagery/next-plate";
        let plate = PlateView {
            state: PlateState::Ready,
            url: Some(url.to_owned()),
            ..Default::default()
        };
        stage.apply_plate(&plate, "next-scene").unwrap();
        let index: usize = stage
            .plate
            .root
            .get_attribute("data-want-i")
            .unwrap()
            .parse()
            .unwrap();
        let image = &stage.plate.images[index];
        let request = stage.plate.root.get_attribute("data-request").unwrap();
        image.set_attribute("data-decoded", &request).unwrap();
        super::activate_plate_request(
            &stage.root,
            &stage.plate.root,
            image,
            &std::rc::Rc::downgrade(&stage.figures),
            &stage.plate.pending_layout,
            &stage.plate.picture_size,
        );
        let push = stage.plate.root.get_attribute("data-push");
        super::activate_plate_request(
            &stage.root,
            &stage.plate.root,
            image,
            &std::rc::Rc::downgrade(&stage.figures),
            &stage.plate.pending_layout,
            &stage.plate.picture_size,
        );
        assert_eq!(stage.plate.root.get_attribute("data-push"), push);
        finish_stage(parent, stage);
    }

    #[test]
    fn late_join_context_is_separate_from_the_active_narrator_caption() {
        let (parent, stage) = mounted_stage();
        stage
            .set_scene_context("A letter lies on the table.")
            .unwrap();
        stage
            .narrator_line(
                1,
                crate::stage_machines::LineState::Done,
                "A reply has arrived.",
            )
            .unwrap();
        assert_eq!(
            stage
                .root
                .query_selector(".tv-scene-context")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("A letter lies on the table.")
        );
        assert_eq!(
            stage.narration.text_content().as_deref(),
            Some("A reply has arrived.")
        );
        finish_stage(parent, stage);
    }

    #[test]
    fn new_scene_plate_keeps_the_current_plate_visible_until_replacement_is_ready() {
        let (parent, stage) = mounted_stage();
        seed_active_plate(&stage);
        let replacement = PlateView {
            state: PlateState::Ready,
            url: Some("/gameplay/imagery/new".to_owned()),
            layout: None,
        };

        stage.set_plate(&replacement, "new-scene", true).unwrap();

        assert_eq!(
            stage.plate.root.get_attribute("data-active").as_deref(),
            Some("0")
        );
        assert_eq!(
            stage.plate.root.get_attribute("data-want-i").as_deref(),
            Some("1")
        );
        assert!(stage.root.has_attribute("data-plate-loaded"));
        finish_stage(parent, stage);
    }

    #[test]
    fn pending_new_scene_keeps_the_current_plate_visible() {
        let (parent, stage) = mounted_stage();
        seed_active_plate(&stage);
        let pending = PlateView {
            state: PlateState::Pending,
            url: None,
            layout: None,
        };

        stage.set_plate(&pending, "new-scene", true).unwrap();

        assert_eq!(
            stage.plate.root.get_attribute("data-active").as_deref(),
            Some("0")
        );
        assert!(stage.root.has_attribute("data-plate-loaded"));
        assert_eq!(
            stage.root.get_attribute("data-plate-state").as_deref(),
            Some("ready")
        );
        finish_stage(parent, stage);
    }

    #[test]
    fn failed_plate_replacement_keeps_the_current_plate_visible() {
        let (parent, stage) = mounted_stage();
        seed_active_plate(&stage);
        stage
            .plate
            .root
            .set_attribute("data-want", "/gameplay/imagery/new")
            .unwrap();
        stage.plate.root.set_attribute("data-want-i", "1").unwrap();
        stage.plate.root.set_attribute("data-request", "1").unwrap();
        stage.plate.images[1]
            .set_attribute("data-src", "/gameplay/imagery/new")
            .unwrap();
        stage.plate.images[1]
            .set_attribute("data-request", "1")
            .unwrap();

        stage.plate.images[1]
            .dispatch_event(&web_sys::Event::new("error").unwrap())
            .unwrap();

        assert_eq!(
            stage.plate.root.get_attribute("data-active").as_deref(),
            Some("0")
        );
        assert!(stage.root.has_attribute("data-plate-loaded"));
        finish_stage(parent, stage);
    }

    #[test]
    fn stale_plate_failure_for_a_reused_asset_path_is_ignored() {
        let (parent, stage) = mounted_stage();
        seed_active_plate(&stage);
        stage
            .plate
            .root
            .set_attribute("data-want", "/gameplay/imagery/new")
            .unwrap();
        stage.plate.root.set_attribute("data-want-i", "1").unwrap();
        stage.plate.root.set_attribute("data-request", "2").unwrap();
        stage.plate.images[1]
            .set_attribute("data-src", "/gameplay/imagery/new")
            .unwrap();
        stage.plate.images[1]
            .set_attribute("data-request", "1")
            .unwrap();

        stage.plate.images[1]
            .dispatch_event(&web_sys::Event::new("error").unwrap())
            .unwrap();

        assert!(!stage.plate.root.has_attribute("data-failed"));
        assert!(stage.root.has_attribute("data-plate-loaded"));
        finish_stage(parent, stage);
    }

    #[test]
    fn sound_gate_offers_start_with_sound_and_continue_without_sound() {
        let (parent, stage) = mounted_stage();
        assert!(stage.root.has_attribute("data-sound-gate"));
        assert_eq!(
            stage.sound_gate.layer.get_attribute("role").as_deref(),
            Some("dialog")
        );
        assert_eq!(
            stage.sound_gate.start.text_content().as_deref(),
            Some("Start with sound")
        );
        assert_eq!(
            stage.sound_gate.skip.text_content().as_deref(),
            Some("Continue without sound")
        );

        let skip_press = web_sys::Event::new("pointerdown").unwrap();
        stage.sound_gate.skip.dispatch_event(&skip_press).unwrap();
        assert!(
            js_sys::Reflect::get(
                skip_press.as_ref(),
                &wasm_bindgen::JsValue::from_str("cancelBubble"),
            )
            .unwrap()
            .as_bool()
            .unwrap()
        );

        let start_press = web_sys::Event::new("pointerdown").unwrap();
        stage.sound_gate.start.dispatch_event(&start_press).unwrap();
        assert!(
            !js_sys::Reflect::get(
                start_press.as_ref(),
                &wasm_bindgen::JsValue::from_str("cancelBubble"),
            )
            .unwrap()
            .as_bool()
            .unwrap()
        );

        stage
            .sound_gate
            .skip
            .dispatch_event(&web_sys::Event::new("click").unwrap())
            .unwrap();

        assert!(!stage.root.has_attribute("data-sound-gate"));
        assert!(stage.sound_gate.layer.parent_element().is_none());
        finish_stage(parent, stage);
    }

    #[test]
    fn display_check_shows_both_advantage_faces_and_the_kept_total_without_the_dc() {
        let check = rpc::AbilityCheckResult {
            die: 16,
            ability_modifier: 1,
            proficiency_bonus: 2,
            total: 19,
            difficulty_class: 15,
            succeeded: true,
            source_revision: "srd521-p6:apothecary:perception:finding-counter".to_owned(),
            faces: vec![16, 4],
            roll_mode: 1,
        };
        let journey = rpc::JourneyView {
            party: vec![rpc::PartyMemberView {
                name: "Brakka".to_owned(),
                ..Default::default()
            }],
            table_log: vec![rpc::TableLine {
                role: rpc::TableRole::Narrator as i32,
                kind: rpc::TableKind::Action as i32,
                speaker: "Roll".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let shown = check_display(
            &check,
            &journey,
            "The bronze ledger bears a seal. More text.",
        )
        .expect("a complete check has a display projection");
        assert_eq!(shown.hero_name, "Brakka");
        assert_eq!(shown.ability_skill, "Wisdom (Perception)");
        assert_eq!(shown.faces, [(16, true), (4, false)]);
        assert_eq!(shown.modifier, 3);
        assert_eq!(shown.total, 19);
        assert!(shown.succeeded);
        assert_eq!(shown.finding, "The bronze ledger bears a seal.");
    }

    #[test]
    fn display_check_uses_roller_index_when_the_latest_hero_action_is_different() {
        let check = rpc::AbilityCheckResult {
            die: 4,
            ability_modifier: -1,
            total: 3,
            succeeded: false,
            source_revision: "srd521-p6:apothecary:perception:finding-counter".to_owned(),
            faces: vec![16, 4],
            roll_mode: -1,
            ..Default::default()
        };
        let journey = rpc::JourneyView {
            party: vec![
                rpc::PartyMemberView {
                    name: "Brakka".to_owned(),
                    ..Default::default()
                },
                rpc::PartyMemberView {
                    name: "Sigrun".to_owned(),
                    ..Default::default()
                },
            ],
            table_log: vec![
                rpc::TableLine {
                    role: rpc::TableRole::Hero as i32,
                    kind: rpc::TableKind::Action as i32,
                    speaker: "Brakka".to_owned(),
                    ..Default::default()
                },
                rpc::TableLine {
                    role: rpc::TableRole::Narrator as i32,
                    hero_index: 1,
                    kind: rpc::TableKind::Action as i32,
                    speaker: "Roll".to_owned(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let shown = check_display(&check, &journey, "Nothing new comes to light.")
            .expect("a complete check has a display projection");
        assert_eq!(shown.hero_name, "Sigrun");
        assert_eq!(shown.faces, [(16, false), (4, true)]);
        assert_eq!(shown.roll_mode, -1);
        assert_eq!(shown.modifier, -1);
        assert!(!shown.succeeded);
    }

    #[test]
    fn display_check_accepts_social_and_dexterity_skill_labels() {
        for (skill, ability, label) in [
            ("acrobatics", "Dexterity", "Acrobatics"),
            ("animal-handling", "Wisdom", "Animal Handling"),
            ("athletics", "Strength", "Athletics"),
            ("deception", "Charisma", "Deception"),
            ("intimidation", "Charisma", "Intimidation"),
            ("performance", "Charisma", "Performance"),
            ("persuasion", "Charisma", "Persuasion"),
            ("sleight-of-hand", "Dexterity", "Sleight of Hand"),
        ] {
            assert_eq!(
                super::check_skill_label(&format!("srd521-p6:hero:{skill}:target")),
                Some((ability, label))
            );
        }
    }

    #[test]
    fn display_check_rejects_an_incomplete_advantage_roll() {
        let check = rpc::AbilityCheckResult {
            die: 16,
            source_revision: "srd521-p6:apothecary:perception:finding-counter".to_owned(),
            faces: vec![16],
            roll_mode: 1,
            ..Default::default()
        };
        assert!(check_display(&check, &rpc::JourneyView::default(), "A clue appears.").is_err());
    }

    #[test]
    fn check_result_replaces_only_the_action_bubble_and_keeps_dialogue_visible() {
        let css = include_str!("live_stage.css");
        assert!(css.contains(".tv-check-result"));
        assert!(!css.contains("[data-check-active] :is(.tv-bubble,.tv-lower-third)"));
    }

    #[test]
    fn end_card_css_hides_scene_chrome_and_keeps_a_single_composition() {
        let css = include_str!("live_stage.css");
        assert!(css.contains(".tv-stage[data-ended] :is("));
        assert!(!css.contains("tv-bigcard-notebook"));
    }

    #[test]
    fn completed_narration_does_not_overlap_a_character_subtitle() {
        let (parent, stage) = mounted_stage();
        let style_node = super::node(
            &stage.root.owner_document().unwrap(),
            &parent,
            "style",
            "",
            "",
        )
        .unwrap();
        style_node.set_text_content(Some(&format!(
            "{}\n{}\n.tv-stagebox{{transition:none!important}}",
            include_str!("live_stage.css"),
            include_str!("live_endings.css")
        )));
        stage
            .root
            .set_attribute("data-scene-state", "awaiting-choice")
            .unwrap();
        stage
            .narrator_line(
                1,
                crate::stage_machines::LineState::Done,
                "The room is quiet.",
            )
            .unwrap();
        stage.set_speaker("character").unwrap();
        let style = web_sys::window()
            .unwrap()
            .get_computed_style(&stage.stagebox)
            .unwrap()
            .unwrap();
        assert_eq!(style.get_property_value("visibility").unwrap(), "hidden");
        stage.set_speaker("narrator").unwrap();
        let style = web_sys::window()
            .unwrap()
            .get_computed_style(&stage.stagebox)
            .unwrap()
            .unwrap();
        assert_eq!(style.get_property_value("visibility").unwrap(), "visible");
        finish_stage(parent, stage);
    }

    #[test]
    fn completed_narration_stays_visible_while_the_table_waits() {
        let css = include_str!("live_stage.css");
        assert!(css.contains(".tv-dm[data-line-state=done] .tv-narration:not(:empty)"));
    }
}
