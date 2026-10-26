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
        Card, CardKind, HERO_ROLE, PlateState, PlateView, SceneFamily, StageActor, shot_of,
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
use crate::live_art::mural_view;
pub(super) use live_scene_stage::SceneGraphView;
#[path = "live_tv_status.rs"]
mod status;
use status::plate_loading_visible;

pub(super) const GENERATED: &str = "/assets/ui/generated";
// PLAY-Q01: the server's QR code of the join link (see gameplay/join_link.rs).
const JOIN_QR: &str = "/gameplay/join-qr.svg";
/// The lobby row always shows every seat of the table (open seats pad a smaller party), all in one
/// row (REF-062).
const MAXIMUM_PARTY_SIZE: usize = 6;
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
    /// The floor and light of the place, as the server declared them.
    pub(super) floor: crate::stage_depth::Staging,
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
    /// Latest view held while both picture frames belong to the current dissolve.
    deferred: RefCell<Option<(PlateView, String)>>,
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
    mural_art: Element,
    /// The scene contents last read for the party mural, so an unchanged view parses nothing.
    mural_source: RefCell<String>,
    art: Element,
    art_previous: Element,
    plate: PlateLayer,
    scene_graph: live_scene_stage::StageLayer,
    /// PLAY-V12: the establishing shot's title card: the adventure title on opening, then the scene title.
    establish_title: Element,
    establish_place: Element,
    lower: LowerThird,
    /// The characters on stage, for the lower third's speaker portrait (found by slot or actor id).
    caption_cast: RefCell<Vec<StageActor>>,
    /// The newest narrator line the stage has heard of, and the one the current scene's entry
    /// began after: only a later line is this scene's narration.
    last_narrator_line: Cell<u64>,
    narration_floor: Cell<u64>,
    /// Recovery context yields once this scene has a current line or check of its own.
    scene_context_retired: Cell<bool>,
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
    /// The kill-shot replay of the fight just finished, played once over everything.
    kill_shot: live_kill_shot::KillShotPlayer,
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

/// Writes an attribute only when its value differs: an equal write still queues a mutation record.
pub(super) fn set_attribute_if_changed(
    element: &Element,
    name: &str,
    value: &str,
) -> Result<(), JsValue> {
    if element.get_attribute(name).as_deref() == Some(value) {
        return Ok(());
    }
    element.set_attribute(name, value)
}

/// Replaces text only when it differs, so an unchanged view leaves the node untouched.
pub(super) fn set_text_if_changed(element: &Element, text: &str) {
    if element.text_content().as_deref() != Some(text) {
        element.set_text_content(Some(text));
    }
}

fn set_hidden(element: &Element, hidden: bool) -> Result<(), JsValue> {
    if element.has_attribute("hidden") == hidden {
        return Ok(());
    }
    if hidden {
        element.set_attribute("hidden", "")
    } else {
        element.remove_attribute("hidden")
    }
}

fn plate_loading_timing(stage: &Element, visible: bool, scene: &str) {
    let Some(indicator) = stage.query_selector(".tv-plate-loading").ok().flatten() else {
        return;
    };
    let id = indicator
        .get_attribute("data-loading-component-id")
        .and_then(|id| id.parse::<u64>().ok())
        .unwrap_or_else(super::presentation_timing::next_component_id);
    super::presentation_timing::component(
        Some(&indicator),
        df_observe::presentation::PresentationComponent::ThinkingLoader,
        id,
        scene,
        if visible {
            df_observe::presentation::ComponentPhase::Opened
        } else {
            df_observe::presentation::ComponentPhase::Hidden
        },
        if visible { "queued" } else { "ready" },
        0,
    );
}

fn sync_plate_loading(stage: &Element, server_pending: Option<bool>, scene: &str) {
    let plate_state = stage.get_attribute("data-plate-server-state");
    if let Some(pending) = server_pending {
        if pending {
            let _ = stage.set_attribute("data-plate-server-pending", "");
        } else {
            let _ = stage.remove_attribute("data-plate-server-pending");
        }
    }
    let visible = plate_loading_visible(
        plate_state.as_deref(),
        stage.has_attribute("data-plate-server-pending"),
    );
    let was_visible = stage.has_attribute("data-plate-loading");
    if visible {
        let _ = stage.set_attribute("data-plate-loading", "");
    } else {
        let _ = stage.remove_attribute("data-plate-loading");
    }
    let Some(indicator) = stage.query_selector(".tv-plate-loading").ok().flatten() else {
        return;
    };
    let active_scene = indicator.get_attribute("data-loading-component-scene");
    if was_visible && (!visible || active_scene.as_deref() != Some(scene)) {
        plate_loading_timing(stage, false, active_scene.as_deref().unwrap_or(scene));
        let _ = indicator.remove_attribute("data-loading-component-id");
        let _ = indicator.remove_attribute("data-loading-component-scene");
    }
    if visible && (!was_visible || active_scene.as_deref() != Some(scene)) {
        let id = super::presentation_timing::next_component_id();
        let _ = indicator.set_attribute("data-loading-component-id", &id.to_string());
        let _ = indicator.set_attribute("data-loading-component-scene", scene);
        plate_loading_timing(stage, true, scene);
    }
}

fn can_retain_plate_for_scene(has_active_plate: bool, plate_loaded: bool) -> bool {
    has_active_plate && plate_loaded
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
    for (button, opted_out) in [(&start, false), (&skip, true)] {
        let (stage, layer) = (stage.clone(), layer.clone());
        let listener = Closure::wrap(Box::new(move |_: Event| {
            if opted_out {
                let _marked = stage.set_attribute("data-sound-opted-out", "");
            }
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
    let shot = if is_scene && stage.has_attribute("data-place-plate-retained") {
        crate::presentation::Shot::Stage
    } else if is_scene {
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
    if is_scene && stage.has_attribute("data-mural") && shot != crate::presentation::Shot::Plate {
        stage.set_attribute("data-arrival-mural", "")?;
    } else {
        stage.remove_attribute("data-arrival-mural")?;
    }
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
    } else if stage.has_attribute("data-arrival-mural") {
        VisualSource::Other
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
    } else if stage.has_attribute("data-arrival-mural") {
        stage
            .query_selector(".tv-mural-art")?
            .and_then(|image| image.get_attribute("src"))
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
    let _focus = refresh_speaker_focus_visibility(stage);
    super::presentation_timing::refresh_component_visibility();
    live_scene_stage::refresh_timing_visibility(stage);
    refresh_establish_component(stage);
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
                    1000,
                );
                if !stage.has_attribute("data-entry-speaker-shown")
                    && let Some(now) = web_sys::window()
                        .and_then(|window| window.performance())
                        .map(|clock| clock.now())
                {
                    let _set = stage.set_attribute("data-entry-picture-ms", &now.to_string());
                    if let Some(started) = stage
                        .get_attribute("data-entry-start-ms")
                        .and_then(|value| value.parse().ok())
                    {
                        super::presentation_timing::entry_gap(
                            &frame,
                            df_observe::presentation::PresentationComponent::PlateIncoming,
                            super::presentation_timing::identity(
                                &image.get_attribute("data-src").unwrap_or_default(),
                            ),
                            &scene,
                            started,
                        );
                    }
                }
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

fn refresh_establish_component(stage: &Element) {
    let Ok(Some(establish)) = stage.query_selector(".tv-establish") else {
        return;
    };
    let scene = stage.get_attribute("data-scene-key").unwrap_or_default();
    let previous_scene = establish
        .get_attribute("data-component-scene")
        .unwrap_or_default();
    let previous_id = establish
        .get_attribute("data-component-id")
        .and_then(|id| id.parse::<u64>().ok());
    if let Some(id) = previous_id
        && previous_scene != scene
    {
        super::presentation_timing::component(
            Some(&establish),
            df_observe::presentation::PresentationComponent::SceneCard,
            id,
            &previous_scene,
            df_observe::presentation::ComponentPhase::Closing,
            "scene-change",
            0,
        );
        super::presentation_timing::schedule_component_transition(
            super::presentation_timing::ComponentTransition {
                element: establish.clone(),
                component_type: df_observe::presentation::PresentationComponent::SceneCard,
                id,
                scene: previous_scene.clone(),
                phase: df_observe::presentation::ComponentPhase::Hidden,
                reason: "scene-change",
                read_ms: 0,
                delay_ms: 0,
            },
        );
        let _removed = establish.remove_attribute("data-component-id");
        let _removed = establish.remove_attribute("data-component-open");
    }
    let shown = stage.is_connected()
        && stage.get_attribute("data-tv-mode").as_deref() == Some("scene")
        && stage.get_attribute("data-shot").as_deref() == Some("establishing")
        && !stage.has_attribute("data-narration-over")
        && !stage.has_attribute("data-stage")
        && !stage.has_attribute("data-ended")
        && !stage.has_attribute("data-sound-gate")
        && stage.get_attribute("data-battlefield-live").as_deref() != Some("true");
    if shown {
        let id = establish
            .get_attribute("data-component-id")
            .and_then(|id| id.parse::<u64>().ok())
            .unwrap_or_else(super::presentation_timing::next_component_id);
        let _set = establish.set_attribute("data-component-id", &id.to_string());
        let _set = establish.set_attribute("data-component-scene", &scene);
        let is_open = establish.get_attribute("data-component-open").as_deref() == Some("true");
        if !is_open {
            let _set = establish.set_attribute("data-component-open", "true");
            let read_ms = crate::stage_machines::reading_time_ms(&format!(
                "{} {}",
                establish
                    .query_selector(".tv-establish-title")
                    .ok()
                    .flatten()
                    .and_then(|e| e.text_content())
                    .unwrap_or_default(),
                establish
                    .query_selector(".tv-establish-place")
                    .ok()
                    .flatten()
                    .and_then(|e| e.text_content())
                    .unwrap_or_default(),
            ));
            super::presentation_timing::component(
                None,
                df_observe::presentation::PresentationComponent::SceneCard,
                id,
                &scene,
                df_observe::presentation::ComponentPhase::Requested,
                "ready",
                read_ms,
            );
            super::presentation_timing::component(
                Some(&establish),
                df_observe::presentation::PresentationComponent::SceneCard,
                id,
                &scene,
                df_observe::presentation::ComponentPhase::Opened,
                "ready",
                read_ms,
            );
            super::presentation_timing::schedule_component_transition(
                super::presentation_timing::ComponentTransition {
                    element: establish,
                    component_type: df_observe::presentation::PresentationComponent::SceneCard,
                    id,
                    scene,
                    phase: df_observe::presentation::ComponentPhase::Readable,
                    reason: "ready",
                    read_ms,
                    delay_ms: 500,
                },
            );
            let scene = stage.get_attribute("data-scene-key").unwrap_or_default();
            let close_id = id;
            let close_element = stage.query_selector(".tv-establish").ok().flatten();
            if let Some(close_element) = close_element {
                super::presentation_timing::schedule_component_transition(
                    super::presentation_timing::ComponentTransition {
                        element: close_element.clone(),
                        component_type: df_observe::presentation::PresentationComponent::SceneCard,
                        id: close_id,
                        scene: scene.clone(),
                        phase: df_observe::presentation::ComponentPhase::Closing,
                        reason: "timeout",
                        read_ms,
                        delay_ms: 3_500,
                    },
                );
                super::presentation_timing::schedule_component_transition(
                    super::presentation_timing::ComponentTransition {
                        element: close_element,
                        component_type: df_observe::presentation::PresentationComponent::SceneCard,
                        id: close_id,
                        scene,
                        phase: df_observe::presentation::ComponentPhase::Hidden,
                        reason: "timeout",
                        read_ms,
                        delay_ms: 5_000,
                    },
                );
            }
        }
    } else if let Some(id) = previous_id
        && establish.get_attribute("data-component-open").as_deref() == Some("true")
    {
        let reason = if scene != previous_scene {
            "scene-change"
        } else {
            "successor"
        };
        let read_ms = 0;
        super::presentation_timing::component(
            Some(&establish),
            df_observe::presentation::PresentationComponent::SceneCard,
            id,
            &previous_scene,
            df_observe::presentation::ComponentPhase::Closing,
            reason,
            read_ms,
        );
        let _set = establish.set_attribute("data-component-open", "false");
        super::presentation_timing::schedule_component_transition(
            super::presentation_timing::ComponentTransition {
                element: establish,
                component_type: df_observe::presentation::PresentationComponent::SceneCard,
                id,
                scene: previous_scene,
                phase: df_observe::presentation::ComponentPhase::Hidden,
                reason,
                read_ms,
                delay_ms: 0,
            },
        );
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
    let roller = journey
        .table_log
        .iter()
        .rev()
        .find(|line| {
            rpc::TableRole::try_from(line.role) == Ok(rpc::TableRole::Narrator)
                && rpc::TableKind::try_from(line.kind) == Ok(rpc::TableKind::Action)
                && line.speaker == "Roll"
        })
        .ok_or_else(|| JsValue::from_str("display check roller is absent"))?;
    let roller_index = usize::try_from(roller.hero_index)
        .map_err(|_| JsValue::from_str("display check roller is invalid"))?;
    let hero_name = journey
        .party
        .get(roller_index)
        .map(|hero| hero.name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| JsValue::from_str("display check roller is absent"))?;
    let intention = roll_intention(&roller.text);
    let modifier = check
        .ability_modifier
        .checked_add(proficiency_modifier)
        .ok_or_else(|| JsValue::from_str("display check modifier is invalid"))?;
    let finding = finding.trim().to_owned();
    if finding.is_empty() {
        return Err(JsValue::from_str("display check finding is absent"));
    }
    Ok(live_stage::CheckDisplay {
        sequence: roller.sequence,
        hero_name,
        intention,
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

fn roll_intention(text: &str) -> String {
    let text = text.trim();
    text.rsplit_once(": ")
        .map_or_else(|| text.to_owned(), |(purpose, _)| purpose.trim().to_owned())
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
        let image = node(document, &frame, "img", "tv-plate-img tv-plate-drift", "")?;
        image.set_attribute("alt", "")?;
        image.set_attribute("decoding", "async")?;
        image.set_attribute("data-i", &index.to_string())?;
        let video_layer = node(document, &frame, "div", "tv-loop tv-plate-video", "")?;
        video_loops.push(live_home_video::HomeLoop::new(document, video_layer)?);
        frames.push(frame);
        images.push(image);
    }
    let focus = node(
        document,
        &zoom,
        "div",
        "tv-speaker-focus tv-plate-drift",
        "",
    )?;
    focus.set_attribute("hidden", "")?;
    node(document, &focus, "div", "tv-focus-subject", "")?;
    node(document, &focus, "div", "tv-focus-rim", "")?;
    let loading = node(
        document,
        stage,
        "div",
        "tv-plate-loading",
        "Updating scene…",
    )?;
    loading.set_attribute("role", "status")?;
    loading.set_attribute("aria-live", "polite")?;
    loading.set_attribute("aria-atomic", "true")?;
    loading.set_attribute("data-layout-id", "plate-loading")?;
    loading.set_attribute("data-layout-owner-id", "plate")?;
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
        deferred: RefCell::new(None),
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
                        "failed",
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
                // Capture the generation before awaiting: the same DOM image can be reused
                // for a newer scene (even with the same URL) while decoding is outstanding.
                let requested_generation = root.get_attribute("data-request");
                let decoded = image.decode();
                let (decode_stage, decode_root, decode_figures) =
                    (stage.clone(), root.clone(), figures.clone());
                let (decode_layout, decode_picture_size) =
                    (Rc::clone(&pending_layout), Rc::clone(&picture_size));
                wasm_bindgen_futures::spawn_local(async move {
                    let decoded = wasm_bindgen_futures::JsFuture::from(decoded).await;
                    if decode_root.get_attribute("data-request") != requested_generation
                        || image.get_attribute("data-request") != requested_generation
                    {
                        return;
                    }
                    if decoded.is_ok() {
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
                        super::live_motion::wake_plate_clock();
                    } else {
                        fail_plate_request(
                            &decode_stage,
                            &decode_root,
                            image.as_ref(),
                            &decode_figures,
                            &decode_layout,
                            &decode_picture_size,
                            "decode-fail",
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
                    "failed",
                );
            }) as Box<dyn FnMut(Event)>)
        };
        image.add_event_listener_with_callback("load", load.as_ref().unchecked_ref())?;
        image.add_event_listener_with_callback("error", error.as_ref().unchecked_ref())?;
        layer.listeners.push((load, error));
    }
    for (index, frame) in layer.frames.iter().enumerate() {
        let (root, frame, event_frame, event_stage) = (
            layer.root.clone(),
            frame.clone(),
            frame.clone(),
            stage.clone(),
        );
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
                    if let Some(image) = frame.query_selector(".tv-plate-img").ok().flatten() {
                        let scene = event_stage
                            .get_attribute("data-scene-key")
                            .unwrap_or_default();
                        if let Some(id) = image
                            .get_attribute("data-component-id")
                            .and_then(|id| id.parse::<u64>().ok())
                        {
                            super::presentation_timing::component(
                                Some(&frame),
                                df_observe::presentation::PresentationComponent::PlateIncoming,
                                id,
                                &scene,
                                df_observe::presentation::ComponentPhase::Readable,
                                "ready",
                                0,
                            );
                        }
                    }
                    let _removed = root.remove_attribute("data-transition");
                    let _removed = root.remove_attribute("data-previous");
                    if let (Some(video_loop), Some(path)) = (
                        video_loops.borrow().get(index),
                        video_paths.borrow().get(index),
                    ) {
                        let _applied = video_loop.apply(path.as_deref());
                    }
                } else if active_index != Some(index) {
                    if let Some(image) = frame.query_selector(".tv-plate-img").ok().flatten()
                        && let Some(id) = image
                            .get_attribute("data-component-id")
                            .and_then(|id| id.parse::<u64>().ok())
                    {
                        let scene =
                            image
                                .get_attribute("data-component-scene")
                                .unwrap_or_else(|| {
                                    event_stage
                                        .get_attribute("data-scene-key")
                                        .unwrap_or_default()
                                });
                        super::presentation_timing::component(
                            Some(&frame),
                            df_observe::presentation::PresentationComponent::PlateOutgoing,
                            id,
                            &scene,
                            df_observe::presentation::ComponentPhase::Hidden,
                            "successor",
                            0,
                        );
                        super::presentation_timing::component(
                            Some(&frame),
                            df_observe::presentation::PresentationComponent::PlateIncoming,
                            id,
                            &scene,
                            df_observe::presentation::ComponentPhase::Hidden,
                            "successor",
                            0,
                        );
                    }
                    if let Some(video_loop) = video_loops.borrow().get(index) {
                        video_loop.release();
                        let _removed = frame.remove_attribute("data-video-ready");
                    }
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
            super::live_motion::wake_plate_clock();
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
                            fail_plate_request(
                                &stage, &root, &image, &figures, &layout, &size, "failed",
                            );
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

const PLATE_LINGER_GATE_MS: f64 = 2500.0;

fn plate_line_is_reading(stage: &Element, now: f64) -> bool {
    if stage.query_selector(".tv-bubble:is([data-line-state=opening],[data-line-state=speaking]),.tv-dm:is([data-line-state=opening],[data-line-state=speaking]),.tv-lower:is([data-line-state=opening],[data-line-state=speaking])").ok().flatten().is_some() { return true; }
    stage
        .get_attribute("data-plate-linger-at")
        .and_then(|at| at.parse::<f64>().ok())
        .is_some_and(|at| now - at < PLATE_LINGER_GATE_MS)
}

fn plate_request_ready(root: &Element, image: &Element) -> bool {
    let request = root.get_attribute("data-request");
    if request.is_none() || root.get_attribute("data-failed") == image.get_attribute("data-src") {
        return false;
    }
    let frame = image.parent_element();
    scene_plate_media_ready(
        image.get_attribute("data-decoded") == request,
        frame
            .as_ref()
            .and_then(|frame| frame.get_attribute("data-video-want"))
            .as_deref(),
        frame
            .as_ref()
            .and_then(|frame| frame.get_attribute("data-video-ready"))
            .as_deref(),
    )
}

fn activate_plate_request(
    stage: &Element,
    root: &Element,
    image: &Element,
    figures: &Weak<Figures>,
    pending_layout: &Rc<RefCell<Option<crate::presentation::PlateGeometry>>>,
    picture_size: &Rc<Cell<Option<(f64, f64)>>>,
) {
    activate_plate_request_at(
        stage,
        root,
        image,
        figures,
        pending_layout,
        picture_size,
        js_sys::Date::now(),
    );
}

fn activate_plate_request_at(
    stage: &Element,
    root: &Element,
    image: &Element,
    figures: &Weak<Figures>,
    pending_layout: &Rc<RefCell<Option<crate::presentation::PlateGeometry>>>,
    picture_size: &Rc<Cell<Option<(f64, f64)>>>,
    now: f64,
) {
    if !plate_request_matches(root, image) {
        return;
    }
    // The owned conductor clock retries after the reading gate expires. Activating a plate
    // also moves bubble anchors and changes caption lanes; finish the current readable line
    // before applying either picture or layout. No independent presentation timer is needed.
    if plate_line_is_reading(stage, now) {
        return;
    }
    let request = root.get_attribute("data-request");
    if request.is_some() && root.get_attribute("data-applied-request") == request {
        return;
    }
    if !plate_request_ready(root, image) {
        return;
    }
    let Some(image_index) = image.get_attribute("data-i") else {
        return;
    };
    let Some(incoming_index) = image_index.parse::<usize>().ok() else {
        return;
    };
    let Some(key) = root.get_attribute("data-want-key") else {
        return;
    };
    let scene = stage.get_attribute("data-scene-key").unwrap_or_default();
    let incoming_id = super::presentation_timing::identity(&key);
    let previous = root
        .get_attribute("data-active")
        .and_then(|active| active.parse::<usize>().ok())
        .filter(|previous| *previous != incoming_index);
    let outgoing = previous.and_then(|previous| {
        root.query_selector(&format!(".tv-plate-frame[data-i=\"{previous}\"]"))
            .ok()
            .flatten()
    });
    if let Some(previous_frame) = &outgoing
        && let Some(previous_image) = previous_frame
            .query_selector(".tv-plate-img")
            .ok()
            .flatten()
        && let Some(previous_id) = previous_image
            .get_attribute("data-component-id")
            .and_then(|id| id.parse::<u64>().ok())
    {
        let previous_scene = previous_image
            .get_attribute("data-component-scene")
            .unwrap_or_else(|| scene.clone());
        super::presentation_timing::component(
            Some(previous_frame),
            df_observe::presentation::PresentationComponent::PlateIncoming,
            previous_id,
            &previous_scene,
            df_observe::presentation::ComponentPhase::Closing,
            "successor",
            0,
        );
        super::presentation_timing::component(
            Some(previous_frame),
            df_observe::presentation::PresentationComponent::PlateOutgoing,
            previous_id,
            &previous_scene,
            df_observe::presentation::ComponentPhase::Opened,
            "ready",
            0,
        );
        super::presentation_timing::component(
            Some(previous_frame),
            df_observe::presentation::PresentationComponent::PlateOutgoing,
            previous_id,
            &previous_scene,
            df_observe::presentation::ComponentPhase::Closing,
            "successor",
            0,
        );
    }
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
    if let Some(previous) = root.get_attribute("data-active") {
        let _set = root.set_attribute("data-previous", &previous);
    }
    let frame = image.parent_element();
    // Resolve the inactive frame's zero opacity before changing its selector. Cached images
    // may decode in the mounting task, before the browser has painted the first frame.
    if let Some(frame) = &frame
        && let Some(frame) = frame.dyn_ref::<web_sys::HtmlElement>()
    {
        let _initial_width = frame.offset_width();
    }
    let _set = root.set_attribute("data-transition", &image_index);
    let _set = root.set_attribute("data-transition-at", &now.to_string());
    let _set = root.set_attribute("data-active", &image_index);
    if let Some(frame) = &frame {
        let _set = image.set_attribute("data-component-id", &incoming_id.to_string());
        let _set = image.set_attribute("data-component-scene", &scene);
        let _set = image.set_attribute("data-component-type", "plate_incoming");
        super::presentation_timing::component(
            Some(frame),
            df_observe::presentation::PresentationComponent::PlateIncoming,
            incoming_id,
            &scene,
            df_observe::presentation::ComponentPhase::Opened,
            "ready",
            0,
        );
        super::presentation_timing::schedule_component_transition(
            super::presentation_timing::ComponentTransition {
                element: frame.clone(),
                component_type: df_observe::presentation::PresentationComponent::PlateIncoming,
                id: incoming_id,
                scene: scene.clone(),
                phase: df_observe::presentation::ComponentPhase::Readable,
                reason: "ready",
                read_ms: 0,
                delay_ms: 1_000,
            },
        );
    }
    if let Some(previous_frame) = outgoing
        && let Some(previous_image) = previous_frame
            .query_selector(".tv-plate-img")
            .ok()
            .flatten()
        && let Some(previous_id) = previous_image
            .get_attribute("data-component-id")
            .and_then(|id| id.parse::<u64>().ok())
    {
        let previous_scene = previous_image
            .get_attribute("data-component-scene")
            .unwrap_or_else(|| scene.clone());
        super::presentation_timing::schedule_component_transition(
            super::presentation_timing::ComponentTransition {
                element: previous_frame.clone(),
                component_type: df_observe::presentation::PresentationComponent::PlateIncoming,
                id: previous_id,
                scene: previous_scene.clone(),
                phase: df_observe::presentation::ComponentPhase::Hidden,
                reason: "successor",
                read_ms: 0,
                delay_ms: 1_000,
            },
        );
        super::presentation_timing::schedule_component_transition(
            super::presentation_timing::ComponentTransition {
                element: previous_frame,
                component_type: df_observe::presentation::PresentationComponent::PlateOutgoing,
                id: previous_id,
                scene: previous_scene,
                phase: df_observe::presentation::ComponentPhase::Hidden,
                reason: "successor",
                read_ms: 0,
                delay_ms: 1_000,
            },
        );
    }
    let bitmap_changed = stage.get_attribute("data-plate-key").as_deref() != Some(key.as_str());
    let _set = stage.set_attribute("data-plate-key", &key);
    let _set = stage.set_attribute("data-plate-state", "ready");
    let _removed = root.remove_attribute("data-failed");
    if stage.get_attribute("data-shot").as_deref() != Some("plate")
        && stage
            .query_selector(".tv-lower[data-line-state=lingering]")
            .ok()
            .flatten()
            .is_some()
    {
        let _retained = stage.set_attribute("data-plate-caption-retained", "");
    }
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
        let has_layout = layout.is_some();
        let _applied = figures.set_plate(layout, size);
        let _focused = refresh_plate_focus(root, &figures);
        if bitmap_changed {
            let _retained = figures.retain_plate_captions_in_named_lane();
        }
        if has_layout {
            let _set = stage.set_attribute("data-plate-layout-key", &key);
        } else {
            let _removed = stage.remove_attribute("data-plate-layout-key");
        }
    }
    let _set = stage.set_attribute("data-plate-loaded", "");
    let _removed = stage.remove_attribute("data-place-plate-retained");
    let _removed = root.remove_attribute("data-loading");
    sync_plate_loading(
        stage,
        None,
        &stage.get_attribute("data-scene-key").unwrap_or_default(),
    );
    super::presentation_timing::media(
        "plate-crossfade",
        &stage.get_attribute("data-scene-key").unwrap_or_default(),
        &key,
        "started",
        1000,
    );
    let _synced = sync_shot(stage);
}

fn fail_plate_request(
    stage: &Element,
    root: &Element,
    image: &Element,
    figures: &Weak<Figures>,
    pending_layout: &Rc<RefCell<Option<crate::presentation::PlateGeometry>>>,
    picture_size: &Rc<Cell<Option<(f64, f64)>>>,
    failure_reason: &'static str,
) {
    if !plate_request_matches(root, image) {
        return;
    }
    let _removed = root.remove_attribute("data-loading");
    sync_plate_loading(
        stage,
        None,
        &stage.get_attribute("data-scene-key").unwrap_or_default(),
    );
    let Some(url) = image.get_attribute("data-src") else {
        return;
    };
    let _set = root.set_attribute("data-failed", &url);
    if let Some(key) = root.get_attribute("data-want-key") {
        super::presentation_timing::component(
            None,
            df_observe::presentation::PresentationComponent::PlateIncoming,
            super::presentation_timing::identity(&key),
            &stage.get_attribute("data-scene-key").unwrap_or_default(),
            df_observe::presentation::ComponentPhase::Failed,
            failure_reason,
            0,
        );
    }
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

/// One persisted display tuning value: `--tune-<name>` from the display tuning file the server
/// inlines into the page (`DF_GAMEPLAY_DISPLAY_TUNING`), as a number; `None` when unset.
pub(super) fn tuning_number(element: &Element, name: &str) -> Option<f64> {
    let style = web_sys::window()?.get_computed_style(element).ok()??;
    let raw = style.get_property_value(&format!("--tune-{name}")).ok()?;
    raw.trim().trim_end_matches('%').parse::<f64>().ok()
}

fn refresh_plate_focus(root: &Element, figures: &Figures) -> Result<(), JsValue> {
    use crate::presentation::CaptionSpeaker;
    refresh_speaker_mask(root, figures)?;
    let Some(zoom) = root.query_selector(".tv-plate-zoom")? else {
        return Ok(());
    };
    let key = zoom.get_attribute("data-focus-key").unwrap_or_default();
    let (speaker, narrator) = match zoom.get_attribute("data-focus-role").as_deref() {
        Some("narrator") => (CaptionSpeaker::Unknown, true),
        Some("hero") => {
            let Ok(index) = key.parse() else {
                return Ok(());
            };
            (CaptionSpeaker::Hero(index), false)
        }
        Some("npc") => (CaptionSpeaker::Npc(&key), false),
        _ => return Ok(()),
    };
    let Some((geometry, (picture_width, picture_height))) = figures.plate_geometry() else {
        return Ok(());
    };
    // Measure the untransformed viewport, never the zoom's already transformed bounds.
    let bounds = root.get_bounding_client_rect();
    let frame = PlateFrame {
        screen: Rect::new(0.0, 0.0, bounds.width(), bounds.height()),
        picture_width,
        picture_height,
    };
    if let Some((x, y)) = crate::stage_conductor::plate_focus(&geometry, speaker, narrator, frame) {
        // A character's line also carries its close-up; the narrator keeps the slow pull-back.
        let shot = (!narrator)
            .then(|| {
                let tuning = crate::stage_conductor::SpeakerTuning::from_lookup(|name| {
                    tuning_number(root, name)
                });
                crate::stage_conductor::speaker_shot(&geometry, speaker, frame, &tuning)
            })
            .flatten()
            .map(|shot| {
                format!(
                    ";--speaker-scale:{:.3};--speaker-pan-x:{:.2}%;--speaker-pan-y:{:.2}%",
                    shot.scale, shot.pan_x, shot.pan_y
                )
            })
            .unwrap_or_default();
        zoom.set_attribute("style", &format!("--head-x:{x}%;--head-y:{y}%{shot}"))?;
    }
    Ok(())
}

fn refresh_speaker_focus_visibility(stage: &Element) -> Result<(), JsValue> {
    let focus = stage.query_selector(".tv-speaker-focus[data-mask-ready]")?;
    let speaking = stage
        .query_selector(".tv-bubble[data-line-state=speaking],.tv-lower[data-line-state=speaking]")?
        .is_some();
    let blocked = stage.get_attribute("data-tv-mode").as_deref() != Some("scene")
        || stage.get_attribute("data-shot").as_deref() != Some("plate")
        || stage
            .get_attribute("data-battle-state")
            .is_some_and(|state| state != "absent")
        || stage.get_attribute("data-battlefield-live").as_deref() == Some("true")
        || stage.has_attribute("data-check-active")
        || stage
            .query_selector(".tv-roll,.tv-dm[data-line-state=speaking]")?
            .is_some();
    if speaking && !blocked && focus.is_some() {
        stage.set_attribute("data-speaker-focus", "on")?;
    } else {
        stage.remove_attribute("data-speaker-focus")?;
    }
    Ok(())
}

fn refresh_speaker_mask(root: &Element, figures: &Figures) -> Result<(), JsValue> {
    let Some(stage) = root.parent_element() else {
        return Ok(());
    };
    let Some(focus) = root.query_selector(".tv-speaker-focus")? else {
        return Ok(());
    };
    let Some(zoom) = root.query_selector(".tv-plate-zoom")? else {
        return Ok(());
    };
    let key = zoom.get_attribute("data-focus-key").unwrap_or_default();
    let role = zoom.get_attribute("data-focus-role").unwrap_or_default();
    let mask = figures.plate_geometry().and_then(|(geometry, _)| {
        let mask = match role.as_str() {
            "hero" => geometry.figure_mask(key.parse().ok(), ""),
            "npc" => geometry.figure_mask(None, &key),
            _ => None,
        }?;
        (!mask.mask_url.is_empty()).then(|| mask.mask_url.clone())
    });
    let active = root.get_attribute("data-active").unwrap_or_default();
    let image = root.query_selector(&format!(".tv-plate-img[data-i='{active}']"))?;
    let plate = image
        .as_ref()
        .and_then(|image| image.get_attribute("data-src"));
    if let (Some(mask), Some(plate), Some(image)) = (mask, plate, image)
        && mask.starts_with(&format!("{plate}/figure/"))
        && let Some(frame) = image.parent_element()
    {
        // One overlay follows the active frame, inheriting its handheld motion. Both the
        // photo and overlay use the same drift class, index and animation clock.
        if focus.parent_element().as_ref() != Some(&frame) {
            frame.append_child(&focus)?;
        }
        focus.set_attribute("data-i", &active)?;
        set_attribute_if_changed(
            &focus,
            "style",
            &format!("--focus-plate:url({plate});--focus-mask:url({mask})"),
        )?;
        focus.set_attribute("data-mask-ready", "")?;
        focus.remove_attribute("hidden")?;
        sync_focus_drift(&image, &focus)?;
    } else {
        focus.remove_attribute("data-mask-ready")?;
        focus.remove_attribute("style")?;
        focus.set_attribute("hidden", "")?;
    }
    refresh_speaker_focus_visibility(&stage)
}

fn sync_focus_drift(image: &Element, focus: &Element) -> Result<(), JsValue> {
    // Web Animations is accessed through browser bindings without expanding web-sys features.
    let animations = |element: &Element| -> Result<js_sys::Array, JsValue> {
        let method = js_sys::Reflect::get(element, &JsValue::from_str("getAnimations"))?;
        let method = method.dyn_into::<js_sys::Function>()?;
        Ok(js_sys::Array::from(&method.call0(element)?))
    };
    let source = animations(image)?.get(0);
    let target = animations(focus)?.get(0);
    if !source.is_undefined() && !target.is_undefined() {
        let time = js_sys::Reflect::get(&source, &JsValue::from_str("currentTime"))?;
        js_sys::Reflect::set(&target, &JsValue::from_str("currentTime"), &time)?;
    }
    Ok(())
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
        (x / width * 100.0).clamp(0.0, 100.0),
        (y / height * 100.0).clamp(0.0, 100.0)
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
        let kill_shot = live_kill_shot::KillShotPlayer::new(document, &root)?;
        let atmosphere = node(document, &root, "div", "tv-atmosphere", "")?;
        let mural = node(document, &root, "div", "tv-mural", "")?;
        mural.set_attribute("aria-hidden", "true")?;
        let mural_art = node(document, &mural, "img", "tv-mural-art", "")?;
        mural_art.set_attribute("alt", "")?;
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
            "0 of 0 heroes ready",
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
        narration.set_attribute("tabindex", "0")?;
        narration.set_attribute("aria-label", "Narration")?;
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
        let mut cards = Vec::with_capacity(MAXIMUM_PARTY_SIZE);
        for _ in 0..MAXIMUM_PARTY_SIZE {
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
            mural_art,
            mural_source: RefCell::new(String::new()),
            art,
            art_previous,
            plate,
            scene_graph,
            establish_title,
            establish_place,
            lower,
            caption_cast: RefCell::new(Vec::new()),
            last_narrator_line: Cell::new(0),
            narration_floor: Cell::new(0),
            scene_context_retired: Cell::new(false),
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
            kill_shot,
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

    pub(super) fn update_mural_wire(&self, contents: &str) -> Result<(), JsValue> {
        if *self.mural_source.borrow() == contents {
            return Ok(());
        }
        *self.mural_source.borrow_mut() = contents.to_owned();
        let mural = mural_view::MuralView::from_scene_contents(contents);
        if let Some(mural) = mural.filter(|mural| mural.state == mural_view::MuralState::Ready) {
            if self.mural_art.get_attribute("src").as_deref() != Some(&mural.url) {
                self.mural_art.set_attribute("src", &mural.url)?;
            }
            self.root.set_attribute("data-mural", "ready")?;
        }
        // The mural belongs to the session, not to this place's scene metadata. A pending,
        // failed or omitted replacement must not discard the last validated party picture.
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

    pub(super) fn update_kill_shot_wire(
        &self,
        shot: Option<&rpc::KillShot>,
    ) -> Result<(), JsValue> {
        self.kill_shot.apply(shot)
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
                self.plate.root.set_attribute("data-loading", "")?;
                sync_plate_loading(
                    &self.root,
                    None,
                    &self
                        .root
                        .get_attribute("data-scene-key")
                        .unwrap_or_default(),
                );
                self.plate
                    .root
                    .set_attribute("data-want-i", &target.to_string())?;
                self.plate.root.set_attribute("data-want-key", &key)?;
                self.plate.root.set_attribute("data-pull", "focus")?;
                image.set_attribute("data-request", &request.to_string())?;
                image.set_attribute("data-src", &url)?;
                image.remove_attribute("data-decoded")?;
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
        }?;
        if !matches!(state, "title" | "closing")
            && self
                .card
                .root
                .get_attribute("data-component-open")
                .as_deref()
                == Some("true")
        {
            let id = self
                .card
                .root
                .get_attribute("data-component-id")
                .and_then(|id| id.parse::<u64>().ok());
            let component = match self
                .card
                .root
                .get_attribute("data-component-type")
                .as_deref()
            {
                Some("title_card") => {
                    Some(df_observe::presentation::PresentationComponent::TitleCard)
                }
                Some("scene_card") => {
                    Some(df_observe::presentation::PresentationComponent::SceneCard)
                }
                _ => None,
            };
            if let (Some(id), Some(component)) = (id, component) {
                let scene = self
                    .root
                    .get_attribute("data-scene-key")
                    .unwrap_or_default();
                super::presentation_timing::component(
                    Some(&self.card.root),
                    component,
                    id,
                    &scene,
                    df_observe::presentation::ComponentPhase::Closing,
                    "successor",
                    0,
                );
                self.card
                    .root
                    .set_attribute("data-component-open", "false")?;
                super::presentation_timing::schedule_component_transition(
                    super::presentation_timing::ComponentTransition {
                        element: self.card.root.clone(),
                        component_type: component,
                        id,
                        scene,
                        phase: df_observe::presentation::ComponentPhase::Hidden,
                        reason: "successor",
                        read_ms: 0,
                        delay_ms: 0,
                    },
                );
            }
        }
        Ok(())
    }

    /// Fills the title or closing card; the stylesheet shows it while the director is in the
    /// matching state.
    pub(super) fn show_card(&self, document: &Document, card: &Card) -> Result<(), JsValue> {
        let shown = &self.card;
        let kind = match card.kind {
            CardKind::Title => "title",
            CardKind::Victory => "victory",
            CardKind::Defeat => "defeat",
            CardKind::End => "end",
        };
        let component = if card.kind == CardKind::Title {
            df_observe::presentation::PresentationComponent::TitleCard
        } else {
            df_observe::presentation::PresentationComponent::SceneCard
        };
        let component_kind = component.label();
        let same_kind = shown.root.get_attribute("data-component-kind").as_deref() == Some(kind);
        let same_content = shown.kicker.text_content().as_deref() == Some(card.kicker.as_str())
            && shown.headline.text_content().as_deref() == Some(card.headline.as_str())
            && shown.line.text_content().as_deref() == Some(card.subline.as_str())
            && shown.place.text_content().as_deref() == Some(card.place.as_str());
        let reuse_component = same_kind && same_content;
        let id = if reuse_component {
            shown
                .root
                .get_attribute("data-component-id")
                .and_then(|id| id.parse::<u64>().ok())
                .unwrap_or_else(super::presentation_timing::next_component_id)
        } else {
            if let (Some(previous_id), Some(previous_type)) = (
                shown
                    .root
                    .get_attribute("data-component-id")
                    .and_then(|id| id.parse::<u64>().ok()),
                shown
                    .root
                    .get_attribute("data-component-type")
                    .as_deref()
                    .and_then(|type_label| match type_label {
                        "title_card" => {
                            Some(df_observe::presentation::PresentationComponent::TitleCard)
                        }
                        "scene_card" => {
                            Some(df_observe::presentation::PresentationComponent::SceneCard)
                        }
                        _ => None,
                    }),
            ) {
                let scene = self
                    .root
                    .get_attribute("data-scene-key")
                    .unwrap_or_default();
                super::presentation_timing::component(
                    Some(&shown.root),
                    previous_type,
                    previous_id,
                    &scene,
                    df_observe::presentation::ComponentPhase::Closing,
                    "replaced",
                    0,
                );
                super::presentation_timing::component(
                    Some(&shown.root),
                    previous_type,
                    previous_id,
                    &scene,
                    df_observe::presentation::ComponentPhase::Hidden,
                    "replaced",
                    0,
                );
            }
            super::presentation_timing::next_component_id()
        };
        let is_new = !reuse_component || !shown.root.has_attribute("data-component-id");
        shown.root.set_attribute("data-kind", kind)?;
        shown
            .root
            .set_attribute("data-component-id", &id.to_string())?;
        shown.root.set_attribute(
            "data-component-scene",
            &self
                .root
                .get_attribute("data-scene-key")
                .unwrap_or_default(),
        )?;
        shown
            .root
            .set_attribute("data-component-type", component_kind)?;
        shown.root.set_attribute("data-component-kind", kind)?;
        shown.root.set_attribute("data-component-open", "true")?;
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
            let personal = df_render::art_contract::player_hero_image(&hero.portrait);
            crest.set_attribute(
                "style",
                &personal
                    .map(|url| format!("background-image:url({url})"))
                    .unwrap_or_default(),
            )?;
            node(document, &item, "b", "tv-bigcard-name", &hero.name)?;
            node(document, &item, "span", "tv-bigcard-role", &hero.role)?;
        }
        let read_text = format!(
            "{} {} {} {}",
            card.kicker, card.headline, card.subline, card.place
        );
        let read_ms = crate::stage_machines::reading_time_ms(&read_text);
        let scene = self
            .root
            .get_attribute("data-scene-key")
            .unwrap_or_default();
        if is_new {
            super::presentation_timing::component(
                None,
                component,
                id,
                &scene,
                df_observe::presentation::ComponentPhase::Requested,
                "ready",
                read_ms,
            );
        }
        super::presentation_timing::component(
            Some(&shown.root),
            component,
            id,
            &scene,
            df_observe::presentation::ComponentPhase::Opened,
            "ready",
            read_ms,
        );
        super::presentation_timing::schedule_component_transition(
            super::presentation_timing::ComponentTransition {
                element: shown.root.clone(),
                component_type: component,
                id,
                scene,
                phase: df_observe::presentation::ComponentPhase::Readable,
                reason: "ready",
                read_ms,
                delay_ms: 0,
            },
        );
        Ok(())
    }

    /// Aim the existing zoom transition at this line's measured head in the displayed bitmap.
    pub(super) fn focus_speaker(
        &self,
        speaker: crate::presentation::CaptionSpeaker<'_>,
        narrator: bool,
    ) -> Result<(), JsValue> {
        use crate::presentation::CaptionSpeaker;
        let Some(zoom) = self.plate.root.query_selector(".tv-plate-zoom")? else {
            return Ok(());
        };
        // Snapshot inherited focus before a later bitmap can change the scene's default.
        // These are the stylesheet's existing fallbacks when no scene head is available yet.
        if !zoom.has_attribute("style")
            && let Some(window) = web_sys::window()
            && let Some(style) = window.get_computed_style(&zoom)?
        {
            let x = style.get_property_value("--head-x")?;
            let y = style.get_property_value("--head-y")?;
            zoom.set_attribute(
                "style",
                &format!(
                    "--head-x:{};--head-y:{}",
                    if x.trim().is_empty() { "50%" } else { x.trim() },
                    if y.trim().is_empty() { "45%" } else { y.trim() },
                ),
            )?;
        }
        let (role, key) = match (narrator, speaker) {
            (true, _) => ("narrator", String::new()),
            (false, CaptionSpeaker::Hero(index)) => ("hero", index.to_string()),
            (false, CaptionSpeaker::Npc(id)) => ("npc", id.to_owned()),
            (false, CaptionSpeaker::Unknown) => ("unknown", String::new()),
        };
        // Retain the target even before art arrives, then reproject when a bitmap applies.
        zoom.set_attribute("data-focus-role", role)?;
        zoom.set_attribute("data-focus-key", &key)?;
        refresh_plate_focus(&self.plate.root, &self.figures)
    }

    /// Which speaker the stage's words belong to; the stylesheet shows the Dungeon Master's
    /// caption or the character's bubble accordingly, never both.
    pub(super) fn set_speaker(&self, speaker: &str) -> Result<(), JsValue> {
        if speaker.is_empty() {
            self.root.remove_attribute("data-speaker")?;
        } else {
            set_attribute_if_changed(&self.root, "data-speaker", speaker)?;
        }
        self.plate_clock_at(js_sys::Date::now())
    }

    /// Decoded requests and active dissolves keep the existing presentation clock alive.
    pub(super) fn plate_needs_clock(&self) -> bool {
        self.plate.root.has_attribute("data-transition")
            || (self.root.has_attribute("data-entry-start-ms")
                && !self.root.has_attribute("data-entry-speaker-ready"))
            || self.plate.images.iter().any(|image| {
                plate_request_matches(&self.plate.root, image)
                    && plate_request_ready(&self.plate.root, image)
                    && self.plate.root.get_attribute("data-applied-request")
                        != self.plate.root.get_attribute("data-request")
            })
    }

    fn plate_clock_at(&self, now: f64) -> Result<(), JsValue> {
        // The existing conductor clock owns recovery when transitionend is lost (for
        // example a background tab). No detached timeout may retire a newer dissolve.
        if self.plate.root.has_attribute("data-transition")
            && status::plate_transition_expired(
                self.plate
                    .root
                    .get_attribute("data-transition-at")
                    .and_then(|at| at.parse::<f64>().ok()),
                now,
            )
        {
            self.plate.root.remove_attribute("data-transition")?;
            self.plate.root.remove_attribute("data-previous")?;
        }
        if !self.plate.root.has_attribute("data-transition") {
            let deferred = self.plate.deferred.borrow_mut().take();
            if let Some((plate, scene)) = deferred {
                self.apply_plate(&plate, &scene)?;
            }
        }
        for image in &self.plate.images {
            activate_plate_request_at(
                &self.root,
                &self.plate.root,
                image,
                &Rc::downgrade(&self.figures),
                &self.plate.pending_layout,
                &self.plate.picture_size,
                now,
            );
        }
        Ok(())
    }

    /// Applies one transition of the narrator line's machine to the Dungeon Master's box: the
    /// text is set as the line opens, never earlier. Returns whether the line opened.
    pub(super) fn narrator_line(
        &self,
        id: u64,
        state: LineState,
        text: &str,
    ) -> Result<bool, JsValue> {
        // A closing predecessor cannot reclaim the box after a roll or a newer caption.
        if id < self.last_narrator_line.get()
            || (!state.is_active()
                && self.narration.get_attribute("data-component-id").as_deref()
                    != Some(&id.to_string()))
        {
            return Ok(false);
        }
        if state.is_active() {
            self.retire_scene_context("successor")?;
        }
        let scene = self
            .root
            .get_attribute("data-scene-key")
            .unwrap_or_default();
        if state == LineState::Lingering {
            self.root
                .set_attribute("data-plate-linger-at", &js_sys::Date::now().to_string())?;
        }
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
        let previous_component = self
            .narration
            .get_attribute("data-component-id")
            .and_then(|id| id.parse::<u64>().ok())
            .filter(|previous_id| *previous_id != id)
            .zip(self.narration.get_attribute("data-component-scene"));
        if let Some((previous_id, previous_scene)) = &previous_component {
            super::presentation_timing::component(
                Some(&self.narration),
                df_observe::presentation::PresentationComponent::Subtitle,
                *previous_id,
                previous_scene,
                df_observe::presentation::ComponentPhase::Closing,
                "successor",
                0,
            );
        }
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
            &scene,
        );
        if let Some((previous_id, previous_scene)) = previous_component {
            super::presentation_timing::component(
                Some(&self.narration),
                df_observe::presentation::PresentationComponent::Subtitle,
                previous_id,
                &previous_scene,
                df_observe::presentation::ComponentPhase::Hidden,
                "successor",
                0,
            );
        }
        let phase = super::live_stage::line_component_phase(state);
        let reason = super::live_stage::line_component_reason(state);
        let read_ms = crate::stage_machines::reading_time_ms(text);
        let created = self
            .narration
            .get_attribute("data-component-id")
            .and_then(|existing| existing.parse::<u64>().ok())
            != Some(id);
        self.narration
            .set_attribute("data-component-id", &id.to_string())?;
        self.narration
            .set_attribute("data-component-scene", &scene)?;
        self.narration
            .set_attribute("data-component-type", "subtitle")?;
        if created {
            super::presentation_timing::component(
                None,
                df_observe::presentation::PresentationComponent::Subtitle,
                id,
                &scene,
                df_observe::presentation::ComponentPhase::Requested,
                "ready",
                read_ms,
            );
        }
        super::presentation_timing::component(
            Some(&self.narration),
            df_observe::presentation::PresentationComponent::Subtitle,
            id,
            &scene,
            phase,
            reason,
            read_ms,
        );
        if state == LineState::Closing {
            super::presentation_timing::schedule_component_transition(
                super::presentation_timing::ComponentTransition {
                    element: self.narration.clone(),
                    component_type: df_observe::presentation::PresentationComponent::Subtitle,
                    id,
                    scene,
                    phase: df_observe::presentation::ComponentPhase::Hidden,
                    reason,
                    read_ms,
                    delay_ms: 500,
                },
            );
        }
        Ok(opened)
    }

    /// Retires prior words when a new scene, roll or character caption owns the display.
    pub(super) fn clear_narration(&self, reason: &'static str) -> Result<(), JsValue> {
        let component_id = self
            .narration
            .get_attribute("data-component-id")
            .and_then(|id| id.parse::<u64>().ok());
        let component_scene = self
            .narration
            .get_attribute("data-component-scene")
            .unwrap_or_default();
        if let Some(id) = component_id {
            super::presentation_timing::component(
                Some(&self.narration),
                df_observe::presentation::PresentationComponent::Subtitle,
                id,
                &component_scene,
                df_observe::presentation::ComponentPhase::Closing,
                reason,
                0,
            );
        }
        if let Some(id) = self
            .narration
            .parent_element()
            .and_then(|caption| caption.get_attribute("data-line-id"))
            .and_then(|id| id.parse::<u64>().ok())
        {
            super::presentation_timing::close_reason(id, reason);
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
        if let Some(id) = component_id {
            super::presentation_timing::component(
                Some(&self.narration),
                df_observe::presentation::PresentationComponent::Subtitle,
                id,
                &component_scene,
                df_observe::presentation::ComponentPhase::Hidden,
                reason,
                0,
            );
        }
        self.narration.remove_attribute("data-component-id")?;
        self.narration.remove_attribute("data-component-scene")?;
        self.narration.remove_attribute("data-component-type")?;
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

    /// Silent late-join context is a fallback until the scene's current line or check claims it.
    pub(super) fn set_scene_context(&self, text: &str) -> Result<(), JsValue> {
        if text.trim().is_empty() || self.scene_context_retired.get() {
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
                let id = super::presentation_timing::next_component_id();
                context.set_attribute("data-component-id", &id.to_string())?;
                context.set_attribute("data-component-type", "scene_card")?;
                context.set_attribute("data-component-open", "true")?;
                context.set_attribute(
                    "data-component-scene",
                    &self
                        .root
                        .get_attribute("data-scene-key")
                        .unwrap_or_default(),
                )?;
                let scene = self
                    .root
                    .get_attribute("data-scene-key")
                    .unwrap_or_default();
                let read_ms = crate::stage_machines::reading_time_ms(text);
                super::presentation_timing::component(
                    None,
                    df_observe::presentation::PresentationComponent::SceneCard,
                    id,
                    &scene,
                    df_observe::presentation::ComponentPhase::Requested,
                    "ready",
                    read_ms,
                );
                super::presentation_timing::component(
                    Some(&context),
                    df_observe::presentation::PresentationComponent::SceneCard,
                    id,
                    &scene,
                    df_observe::presentation::ComponentPhase::Opened,
                    "ready",
                    read_ms,
                );
                super::presentation_timing::schedule_component_transition(
                    super::presentation_timing::ComponentTransition {
                        element: context.clone(),
                        component_type: df_observe::presentation::PresentationComponent::SceneCard,
                        id,
                        scene,
                        phase: df_observe::presentation::ComponentPhase::Readable,
                        reason: "ready",
                        read_ms,
                        delay_ms: 0,
                    },
                );
                context
            }
        };
        context.set_text_content(Some(text));
        Ok(())
    }

    fn retire_scene_context(&self, reason: &'static str) -> Result<(), JsValue> {
        self.scene_context_retired.set(true);
        let Some(context) = self.root.query_selector(".tv-scene-context")? else {
            return Ok(());
        };
        let identity = context
            .get_attribute("data-component-id")
            .and_then(|id| id.parse::<u64>().ok())
            .zip(context.get_attribute("data-component-scene"));
        if let Some((id, scene)) = &identity {
            super::presentation_timing::component(
                Some(&context),
                df_observe::presentation::PresentationComponent::SceneCard,
                *id,
                scene,
                df_observe::presentation::ComponentPhase::Closing,
                reason,
                0,
            );
        }
        context.remove();
        if let Some((id, scene)) = identity {
            super::presentation_timing::component(
                Some(&context),
                df_observe::presentation::PresentationComponent::SceneCard,
                id,
                &scene,
                df_observe::presentation::ComponentPhase::Hidden,
                reason,
                0,
            );
        }
        Ok(())
    }

    pub(super) fn dock(&self) -> &Element {
        &self.dock
    }

    pub(super) fn update_party_images(&self, journey: &rpc::JourneyView) -> Result<(), JsValue> {
        for (index, card) in self.cards.iter().enumerate() {
            let image = journey
                .party
                .get(index)
                .filter(|member| member.character_ready)
                .and_then(|member| df_render::art_contract::player_hero_image(&member.portrait));
            card.portrait.set_attribute(
                "style",
                &image
                    .map(|url| format!("background-image:url({url})"))
                    .unwrap_or_default(),
            )?;
        }
        Ok(())
    }

    pub(super) fn update(&self, model: &TvModel) -> Result<(), JsValue> {
        let lobby = model.mode == TvMode::Lobby;
        if self.root.get_attribute("data-scene-key").as_deref() != Some(&model.scene_key) {
            self.retire_scene_context("scene-change")?;
            self.scene_context_retired.set(false);
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
        // HOME-01: the loop is paired with the still on screen, so its records name it.
        self.home_loop.note_still(&model.art);
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
        set_text_if_changed(&self.title, lobby_title);
        set_text_if_changed(&self.place, &model.place);
        // A scene titled after its place does not repeat the place beneath the title.
        set_hidden(
            &self.place,
            model.place.is_empty() || model.place == *lobby_title,
        )?;
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
        // Open seats belong to the lobby; scenes show every present party member.
        let shown = if lobby {
            MAXIMUM_PARTY_SIZE
        } else {
            model.party.len()
        };
        self.party
            .set_attribute("style", &format!("--seats:{}", shown.max(1)))?;
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
            set_attribute_if_changed(&card.root, "data-state", state)?;
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
                }
                None => {
                    card.root.set_attribute("data-empty", "")?;
                    card.root.set_attribute("data-ready", "false")?;
                    card.name.set_text_content(Some("Open seat"));
                    card.status.set_text_content(Some("Waiting to join"));
                    card.glyph.set_text_content(Some("+"));
                }
            }
        }
        let seats = model.party.len();
        let ready_seats = model.party.iter().filter(|member| member.ready).count();
        let gathered = lobby && seats > 0 && ready_seats == seats;
        let was_gathered = self.roster.has_attribute("data-gathered");
        if gathered {
            self.roster.set_attribute("data-gathered", "")?;
        } else {
            self.roster.remove_attribute("data-gathered")?;
        }
        set_hidden(&self.gathered, !lobby)?;
        if lobby {
            self.gathered
                .set_text_content(Some(&status::lobby_readiness(ready_seats, seats)));
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
        set_hidden(
            &self.establish_place,
            model.place.is_empty() || model.place == *establish_title,
        )?;
        *self.caption_cast.borrow_mut() = model.cast.clone();
        self.set_plate(&model.plate, &model.scene_key, model.narrated)?;
        if lobby {
            self.figures.update(
                &[],
                &model.scene_key,
                model.floor,
                model.family,
                StageMode::Scene,
            )?;
        } else {
            self.figures.update(
                &model.cast,
                &model.scene_key,
                model.floor,
                model.family,
                StageMode::Scene,
            )?;
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
        let scene = self
            .root
            .get_attribute("data-scene-key")
            .unwrap_or_default();
        self.root.set_attribute("data-entry-scene-key", &scene)?;
        self.root.remove_attribute("data-entry-speaker-shown")?;
        self.root.remove_attribute("data-entry-speaker-ready")?;
        self.root.remove_attribute("data-entry-title-cleared-ms")?;
        self.root.remove_attribute("data-entry-start-ms")?;
        self.root.remove_attribute("data-entry-picture-ms")?;
        // A new place releases the old shot; a spot move keeps its loaded people plate.
        if !self.same_place.get() {
            self.root.remove_attribute("data-shot")?;
        }
        if self.drawn.get()
            && let Some(now) = web_sys::window()
                .and_then(|window| window.performance())
                .map(|clock| clock.now())
        {
            self.root
                .set_attribute("data-entry-start-ms", &now.to_string())?;
            // update() already mounted this scene's place art. It is the permitted fallback
            // while a generated plate is pending, and has its own measured dissolve budget.
            self.root
                .set_attribute("data-entry-picture-ms", &now.to_string())?;
            super::presentation_timing::media("scene", &scene, &scene, "scene-change", 0);
        }
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

    /// The conductor asks before starting a character's audio or reading timer. Narration
    /// can establish the scene; a character waits for its picture and title-card exit.
    pub(super) fn speaker_entry_ready(&self) -> Result<bool, JsValue> {
        self.plate_clock_at(js_sys::Date::now())?;
        let Some(now) = web_sys::window()
            .and_then(|window| window.performance())
            .map(|clock| clock.now())
        else {
            return Ok(false);
        };
        self.speaker_entry_ready_at(now)
    }

    fn speaker_entry_ready_at(&self, now: f64) -> Result<bool, JsValue> {
        if self.root.has_attribute("data-entry-speaker-shown")
            || self.root.has_attribute("data-entry-speaker-ready")
        {
            return Ok(true);
        }
        let started = self
            .root
            .get_attribute("data-entry-start-ms")
            .and_then(|value| value.parse::<f64>().ok());
        if started.is_some_and(|started| now - started >= status::ESTABLISHING_BEAT_MS)
            && !self.root.has_attribute("data-entry-title-cleared-ms")
        {
            self.narration_over()?;
            self.root
                .set_attribute("data-entry-title-cleared-ms", &now.to_string())?;
        }
        let picture = self
            .root
            .get_attribute("data-entry-picture-ms")
            .and_then(|value| value.parse::<f64>().ok());
        let title = self
            .root
            .get_attribute("data-entry-title-cleared-ms")
            .and_then(|value| value.parse::<f64>().ok());
        let ready = status::speaker_entry_ready(started, picture, now)
            && (started.is_none()
                || title
                    .is_some_and(|cleared| now - cleared >= status::PLATE_TRANSITION_RECOVERY_MS));
        if ready {
            self.root.set_attribute("data-entry-speaker-ready", "")?;
        }
        Ok(ready)
    }

    /// Check payloads are presented only at their position in the shared conductor queue.
    pub(super) fn begin_check(
        &self,
        check: &super::live_stage::CheckDisplay,
        settled: bool,
    ) -> Result<(), JsValue> {
        self.clear_narration("successor")?;
        self.figures.begin_check(check, settled)?;
        self.retire_scene_context("successor")?;
        self.root.set_attribute("data-check-active", "")?;
        self.root
            .set_attribute("data-check-sequence", &check.sequence.to_string())?;
        refresh_timing_visibility(&self.root);
        Ok(())
    }

    pub(super) fn reveal_check(&self, sequence: u64) -> Result<(), JsValue> {
        self.figures.reveal_check(sequence)?;
        refresh_timing_visibility(&self.root);
        Ok(())
    }

    pub(super) fn finish_check(&self, sequence: u64) -> Result<(), JsValue> {
        self.figures.finish_check(sequence)?;
        if self.root.get_attribute("data-check-sequence").as_deref() == Some(&sequence.to_string())
        {
            self.root.remove_attribute("data-check-active")?;
            self.root.remove_attribute("data-check-sequence")?;
        }
        refresh_timing_visibility(&self.root);
        Ok(())
    }

    pub(super) fn close_check(&self, sequence: u64) -> Result<(), JsValue> {
        self.figures.close_check(sequence)?;
        refresh_timing_visibility(&self.root);
        Ok(())
    }

    /// The character-line adapter routes dialogue to the portrait lane and leaves action callouts
    /// on the figure layer.
    pub(super) fn character_line(
        &self,
        position: usize,
        id: u64,
        state: LineState,
        bubble: &super::live_stage::LineBubble<'_>,
    ) -> Result<bool, JsValue> {
        // Spoken and typed dialogue share one readable, portrait-led caption lane.  The
        // anchored bubbles remain reserved for actions and intent so a character never has two
        // competing dialogue presentations in the same beat.
        if matches!(
            bubble.kind,
            super::live_stage::BubbleKind::Speech | super::live_stage::BubbleKind::Typed
        ) || position == usize::MAX
        {
            if state.is_active() {
                self.clear_narration("successor")?;
                // Action call-outs still use the figure layer and can leave these fallback
                // markers set; clear the actual stage when dialogue takes ownership of the lane.
                self.root.remove_attribute("data-unplaced-line")?;
                self.root.remove_attribute("data-typed-strip")?;
                self.root.remove_attribute("data-typed-strip-closing")?;
            }
            let opened = self
                .lower
                .root
                .get_attribute("data-component-id")
                .and_then(|value| value.parse::<u64>().ok())
                != Some(id);
            self.lower_third(id, bubble.speaker_actor, bubble.speaker, bubble.text, state)?;
            let scene = self
                .root
                .get_attribute("data-scene-key")
                .unwrap_or_default();
            super::presentation_timing::line(
                id,
                state,
                Some(position),
                bubble.text.chars().count() as u32,
                crate::stage_machines::reading_time_ms(bubble.text),
                &scene,
            );
            return Ok(state == LineState::Opening && opened);
        }
        self.figures.set_line(position, id, state, bubble)
    }

    /// Shows what a character says in the lower third, with the speaker's portrait and name, for
    /// every ordinary, plate, and combat-stage dialogue beat.
    pub(super) fn lower_third(
        &self,
        id: u64,
        speaker_actor: crate::presentation::CaptionSpeaker<'_>,
        speaker: &str,
        text: &str,
        state: LineState,
    ) -> Result<(), JsValue> {
        if state.is_active() {
            self.retire_scene_context("successor")?;
        }
        // A predecessor closes after the successor may already own the named lane.
        // Its terminal event must not replace the successor's text, identity or visibility.
        if matches!(
            state,
            LineState::Closing | LineState::Done | LineState::Cancelled
        ) && self
            .lower
            .root
            .get_attribute("data-component-id")
            .as_deref()
            != Some(&id.to_string())
        {
            super::presentation_timing::component(
                None,
                df_observe::presentation::PresentationComponent::Subtitle,
                id,
                &self
                    .root
                    .get_attribute("data-scene-key")
                    .unwrap_or_default(),
                super::live_stage::line_component_phase(state),
                "successor",
                0,
            );
            return Ok(());
        }
        let scene = self
            .root
            .get_attribute("data-scene-key")
            .unwrap_or_default();
        let previous_component = self
            .lower
            .root
            .get_attribute("data-component-id")
            .and_then(|previous_id| previous_id.parse::<u64>().ok())
            .filter(|previous_id| *previous_id != id)
            .zip(self.lower.root.get_attribute("data-component-scene"));
        if let Some((previous_id, previous_scene)) = &previous_component {
            super::presentation_timing::component(
                Some(&self.lower.root),
                df_observe::presentation::PresentationComponent::Subtitle,
                *previous_id,
                previous_scene,
                df_observe::presentation::ComponentPhase::Closing,
                "successor",
                0,
            );
        }
        if state == LineState::Lingering {
            self.root
                .set_attribute("data-plate-linger-at", &js_sys::Date::now().to_string())?;
        }
        if matches!(state, LineState::Closing | LineState::Done) {
            self.root.remove_attribute("data-plate-caption-retained")?;
        }
        let lower = &self.lower;
        if lower.text.text_content().as_deref() != Some(text)
            || lower.name.text_content().as_deref() != Some(speaker)
        {
            lower.name.set_text_content(Some(speaker));
            lower.text.set_text_content(Some(text));
            lower.glyph.set_text_content(Some(&initial(speaker)));
        }
        // The photograph can arrive after the line has opened, so the portrait is checked on every
        // call and changes only when the server's picture for this speaker differs.
        let style =
            crate::presentation::caption_picture(&self.caption_cast.borrow(), speaker_actor)
                .map(|url| format!("background-image:url({url})"))
                .unwrap_or_default();
        if lower.portrait.get_attribute("style").unwrap_or_default() != style {
            lower.portrait.set_attribute("style", &style)?;
        }
        lower.root.set_attribute("data-line-state", state.name())?;
        lower.root.set_attribute(
            "aria-hidden",
            if state.is_active() { "false" } else { "true" },
        )?;
        if let Some((previous_id, previous_scene)) = previous_component {
            super::presentation_timing::component(
                Some(&lower.root),
                df_observe::presentation::PresentationComponent::Subtitle,
                previous_id,
                &previous_scene,
                df_observe::presentation::ComponentPhase::Hidden,
                "successor",
                0,
            );
        }
        let phase = super::live_stage::line_component_phase(state);
        let reason = super::live_stage::line_component_reason(state);
        let read_ms = crate::stage_machines::reading_time_ms(text);
        let created = lower
            .root
            .get_attribute("data-component-id")
            .and_then(|existing| existing.parse::<u64>().ok())
            != Some(id);
        lower
            .root
            .set_attribute("data-component-id", &id.to_string())?;
        lower.root.set_attribute("data-component-scene", &scene)?;
        lower
            .root
            .set_attribute("data-component-type", "subtitle")?;
        if created {
            if state.is_active() && !self.root.has_attribute("data-entry-speaker-shown") {
                self.root.set_attribute("data-entry-speaker-shown", "")?;
                if let Some(shown) = self
                    .root
                    .get_attribute("data-entry-picture-ms")
                    .and_then(|value| value.parse().ok())
                {
                    super::presentation_timing::entry_gap(
                        &lower.root,
                        df_observe::presentation::PresentationComponent::Subtitle,
                        id,
                        &scene,
                        shown,
                    );
                }
            }
            super::presentation_timing::component(
                None,
                df_observe::presentation::PresentationComponent::Subtitle,
                id,
                &scene,
                df_observe::presentation::ComponentPhase::Requested,
                "ready",
                read_ms,
            );
            if state == LineState::Opening {
                super::presentation_timing::schedule_component_transition(
                    super::presentation_timing::ComponentTransition {
                        element: lower.root.clone(),
                        component_type: df_observe::presentation::PresentationComponent::Subtitle,
                        id,
                        scene: scene.clone(),
                        phase: df_observe::presentation::ComponentPhase::Readable,
                        reason: "ready",
                        read_ms,
                        delay_ms: 450,
                    },
                );
            }
        }
        super::presentation_timing::component(
            Some(&lower.root),
            df_observe::presentation::PresentationComponent::Subtitle,
            id,
            &scene,
            phase,
            reason,
            read_ms,
        );
        if matches!(
            state,
            LineState::Closing | LineState::Done | LineState::Cancelled
        ) {
            super::presentation_timing::schedule_component_transition(
                super::presentation_timing::ComponentTransition {
                    element: lower.root.clone(),
                    component_type: df_observe::presentation::PresentationComponent::Subtitle,
                    id,
                    scene,
                    phase: df_observe::presentation::ComponentPhase::Hidden,
                    reason,
                    read_ms,
                    delay_ms: 500,
                },
            );
        }
        Ok(())
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
        if self.root.get_attribute("data-entry-scene-key").as_deref() != Some(scene_key) {
            self.begin_entry(!narrated || !self.drawn.get())?;
        }
        if !self.same_place.get() && self.plate.seen.borrow().as_str() != scene_key {
            self.plate.root.remove_attribute("data-transition")?;
            self.plate.root.remove_attribute("data-transition-at")?;
            *self.plate.deferred.borrow_mut() = None;
            if can_retain_plate_for_scene(
                self.plate.root.has_attribute("data-active"),
                self.root.has_attribute("data-plate-loaded"),
            ) {
                self.root.set_attribute("data-place-plate-retained", "")?;
            } else {
                self.clear_plate(scene_key)?;
            }
        }
        let shown = self.apply_plate(plate, scene_key);
        sync_shot(&self.root)?;
        shown
    }

    /// Retires the picture and media of a place before an arrival exposes its mural.
    fn clear_plate(&self, scene_key: &str) -> Result<(), JsValue> {
        let layer = &self.plate;
        if let Some(active_frame) = layer
            .root
            .get_attribute("data-active")
            .and_then(|active| active.parse::<usize>().ok())
            .and_then(|active| layer.frames.get(active))
            && let Some(image) = active_frame.query_selector(".tv-plate-img")?
            && let Some(id) = image
                .get_attribute("data-component-id")
                .and_then(|id| id.parse::<u64>().ok())
        {
            let previous_scene = image
                .get_attribute("data-component-scene")
                .unwrap_or_else(|| layer.scene.borrow().clone());
            super::presentation_timing::component(
                Some(active_frame),
                df_observe::presentation::PresentationComponent::PlateIncoming,
                id,
                &previous_scene,
                df_observe::presentation::ComponentPhase::Closing,
                "scene-change",
                0,
            );
            super::presentation_timing::component(
                Some(active_frame),
                df_observe::presentation::PresentationComponent::PlateOutgoing,
                id,
                &previous_scene,
                df_observe::presentation::ComponentPhase::Opened,
                "ready",
                0,
            );
            super::presentation_timing::component(
                Some(active_frame),
                df_observe::presentation::PresentationComponent::PlateOutgoing,
                id,
                &previous_scene,
                df_observe::presentation::ComponentPhase::Closing,
                "scene-change",
                0,
            );
            super::presentation_timing::schedule_component_transition(
                super::presentation_timing::ComponentTransition {
                    element: active_frame.clone(),
                    component_type: df_observe::presentation::PresentationComponent::PlateIncoming,
                    id,
                    scene: previous_scene.clone(),
                    phase: df_observe::presentation::ComponentPhase::Hidden,
                    reason: "scene-change",
                    read_ms: 0,
                    delay_ms: 0,
                },
            );
            super::presentation_timing::schedule_component_transition(
                super::presentation_timing::ComponentTransition {
                    element: active_frame.clone(),
                    component_type: df_observe::presentation::PresentationComponent::PlateOutgoing,
                    id,
                    scene: previous_scene,
                    phase: df_observe::presentation::ComponentPhase::Hidden,
                    reason: "scene-change",
                    read_ms: 0,
                    delay_ms: 0,
                },
            );
        }
        self.root.remove_attribute("data-plate-key")?;
        self.root.remove_attribute("data-plate-layout-key")?;
        self.root.remove_attribute("data-plate-loaded")?;
        self.root.remove_attribute("data-place-plate-retained")?;
        layer.root.remove_attribute("data-loading")?;
        layer.root.remove_attribute("data-active")?;
        sync_plate_loading(&self.root, None, scene_key);
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
        *layer.pending_layout.borrow_mut() = None;
        self.figures.set_plate(None, None)?;
        Ok(())
    }

    fn apply_plate(&self, plate: &PlateView, scene_key: &str) -> Result<(), JsValue> {
        let layer = &self.plate;
        self.root
            .set_attribute("data-plate-server-state", plate.state.name())?;
        let current_scene = self.root.get_attribute("data-scene-key").as_deref() == Some(scene_key);
        sync_plate_loading(
            &self.root,
            Some(plate.state == PlateState::Pending && current_scene),
            scene_key,
        );
        let pending_scene_change = plate.state == PlateState::Pending
            && !self.same_place.get()
            && layer.scene.borrow().as_str() != scene_key;
        if pending_scene_change {
            // A dissolve from the previous place must not defer clearing its picture.
            layer.root.remove_attribute("data-transition")?;
            layer.root.remove_attribute("data-transition-at")?;
            *layer.deferred.borrow_mut() = None;
        }
        if layer.root.has_attribute("data-transition") {
            *layer.deferred.borrow_mut() = Some((plate.clone(), scene_key.to_owned()));
            // The conductor clock retires a dissolve whose transitionend was lost, then applies
            // this plate. An idle clock never ticks again on its own, so wake it here.
            super::live_motion::wake_plate_clock();
            return Ok(());
        }
        if layer.seen.borrow().as_str() != scene_key {
            // An in-flight decode belongs to the scene the conductor just left.
            layer.root.remove_attribute("data-request")?;
            layer.root.remove_attribute("data-want")?;
            layer.root.remove_attribute("data-loading")?;
            sync_plate_loading(&self.root, None, scene_key);
        }
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
            if can_retain_plate_for_scene(
                layer.root.has_attribute("data-active"),
                self.root.has_attribute("data-plate-loaded"),
            ) {
                if state == PlateState::Pending
                    && self.root.get_attribute("data-scene-key").as_deref() == Some(scene_key)
                {
                    sync_plate_loading(&self.root, Some(true), scene_key);
                }
                self.root.set_attribute("data-plate-state", state.name())?;
                *layer.seen.borrow_mut() = scene_key.to_owned();
                return Ok(());
            }
            if state == PlateState::Pending
                && self.root.get_attribute("data-scene-key").as_deref() == Some(scene_key)
            {
                sync_plate_loading(&self.root, Some(true), scene_key);
            }
            self.root.set_attribute("data-plate-state", state.name())?;
            self.clear_plate(scene_key)?;
            *layer.seen.borrow_mut() = scene_key.to_owned();
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
                if layout.is_some() {
                    if let Some(key) = self.root.get_attribute("data-plate-key") {
                        self.root.set_attribute("data-plate-layout-key", &key)?;
                    }
                } else {
                    self.root.remove_attribute("data-plate-layout-key")?;
                }
                self.figures
                    .latch_plate_geometry(layout, layer.picture_size.get())?;
                refresh_plate_focus(&layer.root, &self.figures)?;
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
        layer.root.set_attribute("data-loading", "")?;
        sync_plate_loading(&self.root, None, scene_key);
        image.set_attribute("data-request", &request.to_string())?;
        layer.root.set_attribute("data-want-i", &next.to_string())?;
        layer.next.set(1 - next);
        let key = url
            .rsplit('/')
            .next()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| JsValue::from_str("plate URL has no asset key"))?;
        layer.root.set_attribute("data-want-key", key)?;
        let component_id = super::presentation_timing::identity(key);
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
        image.remove_attribute("data-decoded")?;
        image.set_attribute("src", url)?;
        super::presentation_timing::component(
            None,
            df_observe::presentation::PresentationComponent::PlateIncoming,
            component_id,
            scene_key,
            df_observe::presentation::ComponentPhase::Requested,
            if same_scene {
                "replaced"
            } else {
                "scene-change"
            },
            0,
        );
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

#[cfg(any(test, feature = "public-scene-delivery-fixture"))]
pub(super) fn same_revision_art_delivery_reaches_stage_without_waiting_for_the_director() {
    use super::{GameplayViews, RpcConnection, ViewRole, ViewScope, new_client};

    for table_talk_current in [false, true] {
        for talk_advances in [false, true] {
            let document = web_sys::window().unwrap().document().unwrap();
            let parent = document.create_element("section").unwrap();
            document.body().unwrap().append_child(&parent).unwrap();
            let client = new_client(&document, parent.clone(), "display").unwrap();
            let connection = RpcConnection::new(());
            let generation = connection.generation();
            let scope = ViewScope::from_wire(
                Some(&rpc::SessionId {
                    value: Some(vec![1; 16]),
                }),
                Some(&rpc::RunId {
                    value: Some(vec![2; 16]),
                }),
                Some(&rpc::ClientBindingId {
                    value: Some(vec![3; 16]),
                }),
                ViewRole::Display,
            )
            .unwrap();
            let mut views = GameplayViews::new(scope, generation.clone());
            let mut pending = rpc::ViewMessage {
                revision: Some(rpc::SessionRevision {
                    epoch: Some(rpc::RecoveryEpoch { value: Some(1) }),
                    sequence: Some(14),
                }),
                audience: Some(rpc::view_message::Audience::Display(
                    rpc::DisplayGameplayView {
                        narration: "The enforcer has stepped aside.".into(),
                        scene: Some(rpc::GameplayScene {
                            title: "Salt quay".into(),
                            scene_asset: "live-room-photo".into(),
                            plate_state: rpc::PlateState::Pending as i32,
                            ..Default::default()
                        }),
                        journey: Some(rpc::JourneyView {
                            phase: rpc::JourneyPhase::Opening as i32,
                            join_path: "/gameplay/player?room=HS34".into(),
                            join_url: "http://table.test/gameplay/player?room=HS34".into(),
                            party: (0..6)
                                .map(|index| rpc::PartyMemberView {
                                    name: format!("Hero {index}"),
                                    ..Default::default()
                                })
                                .collect(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )),
            };
            views
                .accept_with_art(&generation, scope, pending.clone(), |_, _| false)
                .unwrap();
            if table_talk_current {
                let Some(rpc::view_message::Audience::Display(display)) = &mut pending.audience
                else {
                    unreachable!()
                };
                display.journey.as_mut().unwrap().presentation_sequence = 1;
                views
                    .accept_with_art(&generation, scope, pending.clone(), |_, _| false)
                    .unwrap();
            }
            client.borrow_mut().views = Some(views);
            super::redraw_current(&client).unwrap();
            let stage_root = client.borrow().live.tv.as_ref().unwrap().root().clone();
            assert_eq!(
                stage_root
                    .get_attribute("data-plate-server-state")
                    .as_deref(),
                Some("pending")
            );
            let mut ready = pending;
            let Some(rpc::view_message::Audience::Display(display)) = &mut ready.audience else {
                unreachable!()
            };
            if talk_advances {
                display.journey.as_mut().unwrap().presentation_sequence += 1;
            }
            let scene = display.scene.as_mut().unwrap();
            let plate_path = format!("/gameplay/imagery/{}", "a".repeat(64));
            scene.live_plate = plate_path.clone();
            scene.plate_state = rpc::PlateState::Ready as i32;
            super::receive_view(&client, &generation, scope, ready).unwrap();
            assert_eq!(
                stage_root
                    .get_attribute("data-plate-server-state")
                    .as_deref(),
                Some("ready")
            );
            assert!(
                stage_root
                    .query_selector_all(".tv-plate-img")
                    .unwrap()
                    .length()
                    == 2
            );
            assert!(
                stage_root
                    .query_selector(&format!(".tv-plate-img[src='{plate_path}']"))
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                client
                    .borrow()
                    .views
                    .as_ref()
                    .unwrap()
                    .current_revision()
                    .unwrap()
                    .sequence(),
                14
            );
            client
                .borrow_mut()
                .live
                .tv
                .take()
                .unwrap()
                .dispose()
                .unwrap();
            parent.remove();
        }
    }
}

#[cfg(any(test, feature = "public-scene-delivery-fixture"))]
#[cfg_attr(
    feature = "public-scene-delivery-fixture",
    wasm_bindgen::prelude::wasm_bindgen
)]
pub fn speaker_focus_dom_check() -> Result<String, JsValue> {
    use crate::{
        plate_layout::Side,
        presentation::{CaptionSpeaker, PlateFigureMask, PlateGeometry, PlatePerson},
    };
    let document = web_sys::window()
        .ok_or_else(|| JsValue::from_str("window"))?
        .document()
        .ok_or_else(|| JsValue::from_str("document"))?;
    let parent = document.create_element("section")?;
    document
        .body()
        .ok_or_else(|| JsValue::from_str("body"))?
        .append_child(&parent)?;
    let stage = TvStage::mount(&document, &parent)?;
    let plate = format!("/gameplay/imagery/{}", "a".repeat(64));
    let mask = format!("{plate}/figure/7");
    stage.root.set_attribute("data-tv-mode", "scene").unwrap();
    stage.root.set_attribute("data-shot", "plate").unwrap();
    stage.plate.root.set_attribute("data-active", "1").unwrap();
    stage.plate.images[1]
        .set_attribute("data-src", &plate)
        .unwrap();
    stage
        .lower
        .root
        .set_attribute("data-line-state", "speaking")
        .unwrap();
    stage
        .figures
        .set_plate(
            Some(PlateGeometry {
                bubble_side: Side::Left,
                head_x: 500,
                head_y: 400,
                head_width: 80,
                head_height: 90,
                heroes: Vec::new(),
                actors: Vec::new(),
                figure_masks: vec![PlateFigureMask {
                    person: PlatePerson::Actor("dain".into()),
                    source_width: 1000,
                    source_height: 1000,
                    bounds: [400, 100, 600, 900],
                    head_top: [500, 100],
                    free_side: Side::Left,
                    inferred: false,
                    mask_index: 7,
                    mask_url: mask.clone(),
                }],
            }),
            Some((1000.0, 1000.0)),
        )
        .unwrap();
    stage
        .focus_speaker(CaptionSpeaker::Npc("dain"), false)
        .unwrap();
    assert_eq!(
        stage.root.get_attribute("data-speaker-focus").as_deref(),
        Some("on")
    );
    let focus = stage
        .root
        .query_selector(".tv-speaker-focus")
        .unwrap()
        .unwrap();
    assert_eq!(
        focus.get_attribute("style"),
        Some(format!(
            "--focus-plate:url({plate});--focus-mask:url({mask})"
        ))
    );
    assert_eq!(focus.parent_element(), Some(stage.plate.frames[1].clone()));
    assert!(
        stage.plate.images[1]
            .class_list()
            .contains("tv-plate-drift")
    );
    assert!(focus.class_list().contains("tv-plate-drift"));
    assert_eq!(focus.get_attribute("data-i").as_deref(), Some("1"));
    assert_eq!(focus.query_selector_all(":scope>div").unwrap().length(), 2);
    stage.plate.images[0]
        .set_attribute("data-src", &plate)
        .unwrap();
    stage.plate.root.set_attribute("data-active", "0").unwrap();
    refresh_plate_focus(&stage.plate.root, &stage.figures).unwrap();
    assert_eq!(focus.parent_element(), Some(stage.plate.frames[0].clone()));
    assert_eq!(
        stage
            .root
            .query_selector_all(".tv-speaker-focus")
            .unwrap()
            .length(),
        1
    );
    let wrong_plate = format!("/gameplay/imagery/{}", "b".repeat(64));
    stage.plate.images[0]
        .set_attribute("data-src", &wrong_plate)
        .unwrap();
    refresh_plate_focus(&stage.plate.root, &stage.figures).unwrap();
    assert!(!stage.root.has_attribute("data-speaker-focus"));
    stage.plate.images[0]
        .set_attribute("data-src", &plate)
        .unwrap();
    for (speaker, narrator) in [
        (CaptionSpeaker::Unknown, true),
        (CaptionSpeaker::Npc("unmasked"), false),
        (CaptionSpeaker::Unknown, false),
    ] {
        stage.focus_speaker(speaker, narrator).unwrap();
        assert!(!stage.root.has_attribute("data-speaker-focus"));
        assert!(!focus.has_attribute("style"));
        assert!(focus.has_attribute("hidden"));
    }
    stage
        .focus_speaker(CaptionSpeaker::Npc("dain"), false)
        .unwrap();
    for (attribute, value) in [("data-battle-state", "acting"), ("data-check-active", "")] {
        stage.root.set_attribute(attribute, value).unwrap();
        refresh_speaker_focus_visibility(&stage.root).unwrap();
        assert!(!stage.root.has_attribute("data-speaker-focus"));
        stage.root.remove_attribute(attribute).unwrap();
    }
    let roll = document.create_element("div").unwrap();
    roll.set_class_name("tv-roll");
    stage.root.append_child(&roll).unwrap();
    refresh_speaker_focus_visibility(&stage.root).unwrap();
    assert!(!stage.root.has_attribute("data-speaker-focus"));
    roll.remove();
    stage
        .lower
        .root
        .set_attribute("data-line-state", "lingering")
        .unwrap();
    refresh_speaker_focus_visibility(&stage.root).unwrap();
    assert!(!stage.root.has_attribute("data-speaker-focus"));
    parent.remove();
    Ok("masked NPC on; narrator, unknown, unmasked, battle, check, linger off".into())
}

#[cfg(test)]
mod tests {
    use super::super::rpc;
    use super::{
        PlateState, PlateView, TvStage, can_retain_plate_for_scene, check_display, first_art_asset,
        is_loopback_join_host, plate_asset_key, plate_loading_visible, roll_intention,
        scene_clip_needs_frame_swap, scene_plate_media_ready, scene_video_url,
    };
    use crate::stage_machines::LineState;
    use wasm_bindgen::JsCast;
    fn mounted_stage() -> (web_sys::Element, TvStage) {
        let document = web_sys::window().unwrap().document().unwrap();
        let parent = document.create_element("section").unwrap();
        document.body().unwrap().append_child(&parent).unwrap();
        let stage = TvStage::mount(&document, &parent).unwrap();
        (parent, stage)
    }

    #[test]
    fn hs62_speaker_focus_tracks_masked_npc_and_clears_narrator_unknown_and_unmasked() {
        super::speaker_focus_dom_check().unwrap();
    }

    #[test]
    fn hs48_speaker_origin_survives_late_art_and_tracks_the_displayed_bitmap() {
        use crate::{
            plate_layout::Side,
            presentation::{CaptionSpeaker, PlateActor, PlateGeometry, PlateHero},
        };
        let (parent, stage) = mounted_stage();
        stage
            .plate
            .root
            .set_attribute("style", "position:fixed;width:1000px;height:1000px")
            .unwrap();
        let zoom = stage
            .plate
            .root
            .query_selector(".tv-plate-zoom")
            .unwrap()
            .unwrap();
        stage.focus_speaker(CaptionSpeaker::Unknown, false).unwrap();
        let held_origin = zoom.get_attribute("style");
        assert!(held_origin.is_some());
        stage.focus_speaker(CaptionSpeaker::Hero(2), false).unwrap();
        assert_eq!(zoom.get_attribute("style"), held_origin);
        let mut geometry = PlateGeometry {
            figure_masks: Vec::new(),
            bubble_side: Side::Left,
            head_x: 500,
            head_y: 400,
            head_width: 80,
            head_height: 90,
            heroes: vec![PlateHero {
                index: 2,
                bubble_side: Side::Right,
                head_x: 200,
                head_y: 300,
                head_width: 60,
                head_height: 80,
            }],
            actors: vec![PlateActor {
                actor_id: "dain".into(),
                bubble_side: Side::Left,
                head_x: 800,
                head_y: 350,
                head_width: 70,
                head_height: 80,
            }],
        };
        stage
            .figures
            .set_plate(Some(geometry.clone()), Some((1000.0, 1000.0)))
            .unwrap();
        super::refresh_plate_focus(&stage.plate.root, &stage.figures).unwrap();
        assert_eq!(
            zoom.get_attribute("style")
                .as_deref()
                .and_then(|style| style.split(";--speaker").next()),
            Some("--head-x:20%;--head-y:30%")
        );
        stage
            .focus_speaker(CaptionSpeaker::Npc("dain"), false)
            .unwrap();
        assert_eq!(
            zoom.get_attribute("style")
                .as_deref()
                .and_then(|style| style.split(";--speaker").next()),
            Some("--head-x:80%;--head-y:35%")
        );
        geometry.actors[0].head_x = 700;
        stage
            .figures
            .set_plate(Some(geometry.clone()), Some((1000.0, 1000.0)))
            .unwrap();
        super::refresh_plate_focus(&stage.plate.root, &stage.figures).unwrap();
        assert_eq!(
            zoom.get_attribute("style")
                .as_deref()
                .and_then(|style| style.split(";--speaker").next()),
            Some("--head-x:70%;--head-y:35%")
        );
        stage.focus_speaker(CaptionSpeaker::Unknown, false).unwrap();
        geometry.actors[0].head_x = 600;
        stage
            .figures
            .set_plate(Some(geometry), Some((1000.0, 1000.0)))
            .unwrap();
        super::refresh_plate_focus(&stage.plate.root, &stage.figures).unwrap();
        assert_eq!(
            zoom.get_attribute("style")
                .as_deref()
                .and_then(|style| style.split(";--speaker").next()),
            Some("--head-x:70%;--head-y:35%")
        );
        stage
            .focus_speaker(CaptionSpeaker::Npc("missing"), false)
            .unwrap();
        assert_eq!(
            zoom.get_attribute("style")
                .as_deref()
                .and_then(|style| style.split(";--speaker").next()),
            Some("--head-x:70%;--head-y:35%")
        );
        stage.focus_speaker(CaptionSpeaker::Unknown, true).unwrap();
        assert_eq!(
            zoom.get_attribute("style")
                .as_deref()
                .and_then(|style| style.split(";--speaker").next()),
            Some("--head-x:50%;--head-y:40%")
        );
        finish_stage(parent, stage);
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

    #[test]
    fn same_revision_art_delivery_reaches_stage_without_waiting_for_the_director() {
        super::same_revision_art_delivery_reaches_stage_without_waiting_for_the_director();
    }

    #[test]
    fn idle_clock_stale_transition_applies_deferred_art_and_completes_title_entry() {
        let (parent, stage) = mounted_stage();
        stage.root.set_attribute("data-tv-mode", "scene").unwrap();
        stage.root.set_attribute("data-scene-key", "entry").unwrap();
        *stage.plate.seen.borrow_mut() = "entry".into();
        stage
            .root
            .set_attribute("data-plate-state", "pending")
            .unwrap();
        stage
            .root
            .set_attribute("data-shot", "establishing")
            .unwrap();
        stage.narration_over().unwrap();
        stage
            .plate
            .root
            .set_attribute("data-transition", "0")
            .unwrap();
        stage
            .plate
            .root
            .set_attribute("data-transition-at", "10000")
            .unwrap();
        assert!(!super::super::live_motion::clock_needs_work(false, None));
        stage
            .apply_plate(
                &PlateView {
                    state: PlateState::Ready,
                    url: Some("/gameplay/imagery/ready".into()),
                    ..Default::default()
                },
                "entry",
            )
            .unwrap();
        assert!(stage.plate.deferred.borrow().is_some());
        assert!(super::super::live_motion::clock_needs_work(
            false,
            Some(&stage)
        ));
        stage.plate_clock_at(11000.0).unwrap();
        assert!(stage.plate.deferred.borrow().is_some());
        stage.plate_clock_at(11100.0).unwrap();
        assert!(stage.plate.deferred.borrow().is_none());
        let image = stage
            .plate
            .images
            .iter()
            .find(|image| {
                image.get_attribute("data-src").as_deref() == Some("/gameplay/imagery/ready")
            })
            .unwrap();
        let request = image.get_attribute("data-request").unwrap();
        image.set_attribute("data-decoded", &request).unwrap();
        stage.plate_clock_at(11200.0).unwrap();
        assert!(stage.root.has_attribute("data-plate-loaded"));
        assert_eq!(
            stage.root.get_attribute("data-shot").as_deref(),
            Some("plate")
        );
        assert!(stage.root.has_attribute("data-narration-over"));
        assert_eq!(
            stage.plate.root.get_attribute("data-applied-request"),
            Some(request)
        );
        finish_stage(parent, stage);
    }

    fn finish_stage(parent: web_sys::Element, stage: TvStage) {
        stage.dispose().unwrap();
        parent.remove();
    }

    #[test]
    fn same_scene_art_updates_preserve_entry_deadline_and_first_speaker_waits_for_title_exit() {
        let (parent, stage) = mounted_stage();
        stage.root.set_attribute("data-scene-key", "entry").unwrap();
        stage
            .root
            .set_attribute("data-entry-scene-key", "entry")
            .unwrap();
        stage
            .root
            .set_attribute("data-entry-start-ms", "1000")
            .unwrap();
        stage
            .root
            .set_attribute("data-entry-picture-ms", "1500")
            .unwrap();
        stage.drawn.set(true);
        stage
            .plate
            .root
            .set_attribute("data-transition", "0")
            .unwrap();
        stage
            .plate
            .root
            .set_attribute("data-transition-at", "1000")
            .unwrap();
        stage
            .set_plate(
                &PlateView {
                    state: PlateState::Ready,
                    url: Some("/gameplay/imagery/ready".into()),
                    ..Default::default()
                },
                "entry",
                true,
            )
            .unwrap();
        assert_eq!(
            stage.root.get_attribute("data-entry-start-ms").as_deref(),
            Some("1000")
        );
        assert!(!stage.speaker_entry_ready_at(5999.0).unwrap());
        assert!(!stage.speaker_entry_ready_at(6000.0).unwrap());
        assert!(stage.root.has_attribute("data-narration-over"));
        assert!(!stage.speaker_entry_ready_at(7099.0).unwrap());
        assert!(stage.speaker_entry_ready_at(7100.0).unwrap());
        // Once a speaker has entered, later art in this same scene cannot reclose its lane.
        stage
            .root
            .set_attribute("data-entry-speaker-shown", "")
            .unwrap();
        stage
            .root
            .set_attribute("data-entry-picture-ms", "8000")
            .unwrap();
        assert!(stage.speaker_entry_ready_at(8000.0).unwrap());
        finish_stage(parent, stage);
    }

    #[test]
    fn delayed_plate_and_missing_art_fallback_release_entry_without_reclosing_same_scene() {
        for picture in [8_000.0, 1_000.0] {
            let (parent, stage) = mounted_stage();
            stage.root.set_attribute("data-tv-mode", "scene").unwrap();
            stage
                .root
                .set_attribute("data-entry-start-ms", "1000")
                .unwrap();
            stage
                .root
                .set_attribute("data-entry-picture-ms", &picture.to_string())
                .unwrap();
            assert!(super::super::live_motion::clock_needs_work(
                false,
                Some(&stage)
            ));
            assert!(!stage.speaker_entry_ready_at(6_000.0).unwrap());
            let ready_at = if picture == 8_000.0 { 9_100.0 } else { 7_100.0 };
            assert!(!stage.speaker_entry_ready_at(ready_at - 1.0).unwrap());
            assert!(stage.speaker_entry_ready_at(ready_at).unwrap());
            assert!(!super::super::live_motion::clock_needs_work(
                false,
                Some(&stage)
            ));
            stage
                .root
                .set_attribute("data-entry-picture-ms", "15000")
                .unwrap();
            assert!(stage.speaker_entry_ready_at(15_000.0).unwrap());
            finish_stage(parent, stage);
        }
    }

    #[test]
    fn hero_typing_and_speaking_intents_use_measured_plate_heads_above_hidden_figures() {
        use crate::{
            plate_layout::Side,
            presentation::{ActorSlot, PlateGeometry, PlateHero, SceneFamily, StageActor},
            stage_conductor::IntentKind,
        };

        let (parent, stage) = mounted_stage();
        let document = parent.owner_document().unwrap();
        let style = document.create_element("style").unwrap();
        style.set_text_content(Some(&format!(
            "{}\n{}\n{}",
            include_str!("live_tv.css"),
            include_str!("live_stage.css"),
            include_str!("live_plate.css")
        )));
        parent.append_child(&style).unwrap();
        seed_active_plate(&stage);
        stage
            .figures
            .update(
                &[StageActor {
                    slot: ActorSlot::Hero(0),
                    actor_id: "hero-1".into(),
                    name: "Brakka".into(),
                    role: "Hero".into(),
                    sprite: String::new(),
                    portrait: String::new(),
                    hit_points: Some((10, 13)),
                    speaking: false,
                    down: false,
                    live_sprite: None,
                }],
                "intent-scene",
                crate::stage_depth::DEFAULT,
                SceneFamily::Interior,
                super::StageMode::Scene,
            )
            .unwrap();
        stage
            .root
            .set_attribute("data-battle-state", "absent")
            .unwrap();
        stage.root.remove_attribute("data-sound-gate").unwrap();
        let figure = stage
            .root
            .query_selector(".tv-fig[data-slot=hero-1]")
            .unwrap()
            .unwrap();
        let body = figure.query_selector(".tv-fig-body").unwrap().unwrap();
        let window = web_sys::window().unwrap();
        let mut previous_left = None;
        for (head_x, kind) in [(250, IntentKind::Typing), (650, IntentKind::Speaking)] {
            stage
                .figures
                .set_plate(
                    Some(PlateGeometry {
                        figure_masks: Vec::new(),
                        bubble_side: Side::Right,
                        head_x: 900,
                        head_y: 200,
                        head_width: 50,
                        head_height: 80,
                        heroes: vec![PlateHero {
                            index: 0,
                            bubble_side: Side::Right,
                            head_x,
                            head_y: 350,
                            head_width: 50,
                            head_height: 80,
                        }],
                        actors: vec![],
                    }),
                    Some((1920.0, 1080.0)),
                )
                .unwrap();
            stage
                .figures
                .set_intent(0, 21, LineState::Composing, kind, "Brakka")
                .unwrap();
            let loader = stage
                .root
                .query_selector(".tv-plate-bubbles .tv-intent")
                .unwrap()
                .unwrap();
            let host = loader.parent_element().unwrap();
            assert!(!host.has_attribute("hidden"));
            assert_eq!(
                window
                    .get_computed_style(&body)
                    .unwrap()
                    .unwrap()
                    .get_property_value("visibility")
                    .unwrap(),
                "hidden"
            );
            assert_eq!(
                window
                    .get_computed_style(&loader)
                    .unwrap()
                    .unwrap()
                    .get_property_value("visibility")
                    .unwrap(),
                "visible"
            );
            let bounds = loader.get_bounding_client_rect();
            assert!(bounds.width() > 0.0 && bounds.height() > 0.0);
            assert!(host.has_attribute("data-tail"));
            if let Some(left) = previous_left {
                assert!(
                    bounds.left() > left + 100.0,
                    "the intent follows the measured head, not the cut-out"
                );
            }
            previous_left = Some(bounds.left());
            assert_eq!(
                loader.get_attribute("data-intent").as_deref(),
                Some(if matches!(kind, IntentKind::Speaking) {
                    "speaking"
                } else {
                    "typing"
                })
            );
        }
        stage
            .figures
            .set_intent(0, 21, LineState::Done, IntentKind::Speaking, "Brakka")
            .unwrap();
        assert!(stage.root.query_selector(".tv-intent").unwrap().is_none());
        finish_stage(parent, stage);
    }

    #[test]
    fn six_hero_scene_renders_every_party_card_and_figure_and_hud_card() {
        use super::{TvMember, TvMode, TvModel};
        use crate::presentation::{ActorSlot, SceneFamily, StageActor};
        let (parent, stage) = mounted_stage();
        let document = parent.owner_document().unwrap();
        let style = document.create_element("style").unwrap();
        style.set_text_content(Some(&format!(
            "{}\n{}\n{}",
            include_str!("live_tv.css"),
            include_str!("live_stage.css"),
            include_str!("live_plate.css")
        )));
        parent.append_child(&style).unwrap();
        let cast = (0..6)
            .map(|index| StageActor {
                slot: ActorSlot::Hero(index),
                actor_id: String::new(),
                name: format!("Accepted hero {}", index + 1),
                role: "Hero".into(),
                sprite: String::new(),
                portrait: format!(
                    "/gameplay/imagery/{}",
                    char::from(b'a' + index as u8).to_string().repeat(64)
                ),
                hit_points: Some((10, 13)),
                speaking: false,
                down: false,
                live_sprite: None,
            })
            .collect::<Vec<_>>();
        let model = TvModel {
            mode: TvMode::Scene,
            title: "Six heroes".into(),
            lede: String::new(),
            speaker: None,
            room_code: "SIX".into(),
            join_host: "table.test".into(),
            party: cast
                .iter()
                .map(|hero| TvMember {
                    name: hero.name.clone(),
                    status: "HP 10/13".into(),
                    ready: true,
                })
                .collect(),
            art: String::new(),
            art_fallback: "",
            art_source: df_observe::presentation::VisualSource::Other,
            cast,
            place: "A quiet room".into(),
            rules_note: String::new(),
            choices: Vec::new(),
            choice_hints_hidden: true,
            family: SceneFamily::Interior,
            floor: crate::stage_depth::DEFAULT,
            scene_key: "six-hero-scene".into(),
            visual_scene_id: "six-hero-scene".into(),
            home: None,
            home_video: None,
            plate: PlateView::default(),
            adventure_name: String::new(),
            narrated: false,
            end: None,
            check: None,
        };
        stage.update(&model).unwrap();
        // A same-phase update must retain all seats and their standing places.
        stage.update(&model).unwrap();
        assert_eq!(
            stage
                .party
                .query_selector_all(".tv-card:not([hidden])")
                .unwrap()
                .length(),
            6
        );
        assert_eq!(
            stage
                .root
                .query_selector_all(".tv-fig[data-slot^=hero][data-present=true]")
                .unwrap()
                .length(),
            6
        );
        assert_eq!(
            stage
                .root
                .query_selector_all(".tv-hud-card[data-slot^=hero][data-present=true]")
                .unwrap()
                .length(),
            6
        );
        for (index, card) in stage.cards.iter().enumerate() {
            assert_eq!(card.name.text_content().unwrap(), model.party[index].name);
            assert_eq!(card.status.text_content().as_deref(), Some("HP 10/13"));
        }
        stage
            .lower_third(
                1,
                crate::presentation::CaptionSpeaker::Hero(5),
                "Accepted hero 6",
                "All six of us are here.",
                LineState::Speaking,
            )
            .unwrap();
        let mut previous_right = 0.0;
        for card in &stage.cards {
            let bounds = card.root.get_bounding_client_rect();
            assert!(bounds.width() > 0.0);
            assert!(bounds.left() >= previous_right);
            previous_right = bounds.right();
        }
        let party = stage.party.get_bounding_client_rect();
        let dialogue = stage.lower.root.get_bounding_client_rect();
        assert!(
            dialogue.bottom() <= party.top(),
            "dialogue must clear the compact party strip"
        );
        finish_stage(parent, stage);
    }

    #[test]
    fn ready_plate_suppresses_stale_loading_state() {
        assert!(!plate_loading_visible(Some("ready"), true));
        assert!(!plate_loading_visible(Some("failed"), true));
        assert!(plate_loading_visible(Some("pending"), true));
        assert!(!plate_loading_visible(Some("pending"), false));
    }

    #[test]
    fn terminal_plate_clears_loading_before_an_active_dissolve_finishes() {
        for state in [PlateState::Ready, PlateState::Failed] {
            let (parent, stage) = mounted_stage();
            stage.root.set_attribute("data-scene-key", "quay").unwrap();
            stage
                .apply_plate(
                    &PlateView {
                        state: PlateState::Pending,
                        update_pending: true,
                        ..Default::default()
                    },
                    "quay",
                )
                .unwrap();
            assert!(stage.root.has_attribute("data-plate-loading"));
            stage
                .plate
                .root
                .set_attribute("data-transition", "focus")
                .unwrap();
            stage.plate.root.set_attribute("data-loading", "").unwrap();
            stage
                .apply_plate(
                    &PlateView {
                        state,
                        // Old same-revision metadata and client decode cannot extend this notice.
                        update_pending: true,
                        url: (state == PlateState::Ready)
                            .then(|| "/gameplay/imagery/ready".to_owned()),
                        ..Default::default()
                    },
                    "quay",
                )
                .unwrap();
            assert!(!stage.root.has_attribute("data-plate-loading"));
            assert!(!stage.root.has_attribute("data-plate-server-pending"));
            assert!(stage.plate.deferred.borrow().is_some());
            finish_stage(parent, stage);
        }
    }

    #[test]
    fn a_loaded_place_plate_remains_available_for_a_new_scene() {
        assert!(!can_retain_plate_for_scene(false, true));
        assert!(!can_retain_plate_for_scene(true, false));
        assert!(can_retain_plate_for_scene(true, true));
        assert_eq!(super::status::lobby_readiness(2, 4), "2 of 4 heroes ready");
    }

    #[test]
    fn predecessor_done_cannot_replace_the_successors_unplaced_named_caption() {
        use super::super::live_stage::{BubbleKind, LineBubble, StageMode};
        use crate::presentation::{ActorSlot, PlateGeometry, SceneFamily, StageActor};
        let (parent, stage) = mounted_stage();
        stage.root.set_attribute("data-shot", "plate").unwrap();
        stage
            .figures
            .update(
                &[StageActor {
                    actor_id: String::new(),
                    slot: ActorSlot::Hero(0),
                    name: "Brakka".into(),
                    role: "hero".into(),
                    sprite: String::new(),
                    portrait: String::new(),
                    hit_points: None,
                    speaking: false,
                    down: false,
                    live_sprite: None,
                }],
                "scene",
                crate::stage_depth::DEFAULT,
                SceneFamily::Exterior,
                StageMode::Scene,
            )
            .unwrap();
        stage
            .figures
            .set_plate(
                Some(PlateGeometry {
                    figure_masks: Vec::new(),
                    actors: Vec::new(),
                    bubble_side: crate::plate_layout::Side::Right,
                    head_x: 500,
                    head_y: 200,
                    head_width: 100,
                    head_height: 100,
                    heroes: vec![],
                }),
                Some((1920.0, 1080.0)),
            )
            .unwrap();
        let host = stage
            .root
            .query_selector(".tv-plate-bubbles>.tv-fig-bubbles")
            .unwrap()
            .unwrap();
        host.set_attribute("style", "width:10000px;height:10000px")
            .unwrap();
        let previous = LineBubble {
            kind: BubbleKind::Typed,
            speaker_actor: crate::presentation::CaptionSpeaker::Unknown,
            speaker: "Brakka",
            text: "Earlier line",
            voiced: false,
        };
        let text = format!("Courier {}", "a".repeat(324));
        assert_eq!(text.chars().count(), 332);
        let current = LineBubble {
            kind: BubbleKind::Typed,
            speaker_actor: crate::presentation::CaptionSpeaker::Unknown,
            speaker: "Brakka",
            text: &text,
            voiced: false,
        };
        stage
            .character_line(0, 1, LineState::Opening, &previous)
            .unwrap();
        stage
            .character_line(0, 2, LineState::Opening, &current)
            .unwrap();
        stage
            .character_line(0, 1, LineState::Closing, &previous)
            .unwrap();
        stage
            .character_line(0, 2, LineState::Speaking, &current)
            .unwrap();
        stage
            .character_line(0, 1, LineState::Done, &previous)
            .unwrap();
        assert_eq!(
            stage.lower.text.text_content().as_deref(),
            Some(text.as_str())
        );
        assert_eq!(stage.lower.name.text_content().as_deref(), Some("Brakka"));
        assert_eq!(
            stage
                .lower
                .root
                .get_attribute("data-component-id")
                .as_deref(),
            Some("2")
        );
        assert_eq!(
            stage.lower.root.get_attribute("data-line-state").as_deref(),
            Some("speaking")
        );
        assert!(!stage.root.has_attribute("data-unplaced-line"));
        let npc = LineBubble {
            kind: BubbleKind::Speech,
            speaker_actor: crate::presentation::CaptionSpeaker::Unknown,
            speaker: "Courier",
            text: "An earlier NPC reply",
            voiced: false,
        };
        stage
            .character_line(2, 3, LineState::Opening, &npc)
            .unwrap();
        stage
            .character_line(0, 4, LineState::Opening, &current)
            .unwrap();
        stage
            .character_line(0, 4, LineState::Speaking, &current)
            .unwrap();
        stage
            .character_line(2, 3, LineState::Closing, &npc)
            .unwrap();
        stage.character_line(2, 3, LineState::Done, &npc).unwrap();
        assert_eq!(
            stage
                .lower
                .root
                .get_attribute("data-component-id")
                .as_deref(),
            Some("4")
        );
        assert!(!stage.root.has_attribute("data-unplaced-line"));
        assert!(
            host.query_selector("[data-line-id=\"4\"]")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            stage.lower.text.text_content().as_deref(),
            Some(text.as_str())
        );
        finish_stage(parent, stage);
    }

    #[test]
    fn character_dialogue_uses_the_portrait_lane_without_a_floating_bubble() {
        use super::super::live_stage::{BubbleKind, LineBubble, StageMode};
        use crate::presentation::{ActorSlot, SceneFamily, StageActor};
        let (parent, stage) = mounted_stage();
        stage
            .figures
            .update(
                &[StageActor {
                    actor_id: "courier".into(),
                    slot: ActorSlot::Npc(0),
                    name: "Courier".into(),
                    role: "npc".into(),
                    sprite: String::new(),
                    portrait: String::new(),
                    hit_points: None,
                    speaking: false,
                    down: false,
                    live_sprite: None,
                }],
                "scene",
                crate::stage_depth::DEFAULT,
                SceneFamily::Exterior,
                StageMode::Scene,
            )
            .unwrap();
        let line = LineBubble {
            kind: BubbleKind::Speech,
            speaker_actor: crate::presentation::CaptionSpeaker::Unknown,
            speaker: "Courier",
            text: "You can call me the courier.",
            voiced: true,
        };
        stage
            .root
            .set_attribute("data-unplaced-line", "41")
            .unwrap();
        stage.root.set_attribute("data-typed-strip", "41").unwrap();
        stage
            .root
            .set_attribute("data-typed-strip-closing", "")
            .unwrap();
        assert!(
            stage
                .character_line(2, 42, LineState::Opening, &line)
                .unwrap()
        );
        assert!(!stage.root.has_attribute("data-unplaced-line"));
        assert!(!stage.root.has_attribute("data-typed-strip"));
        assert!(!stage.root.has_attribute("data-typed-strip-closing"));
        assert_eq!(stage.lower.name.text_content().as_deref(), Some("Courier"));
        assert_eq!(stage.lower.text.text_content().as_deref(), Some(line.text));
        assert_eq!(
            stage
                .lower
                .root
                .get_attribute("data-component-type")
                .as_deref(),
            Some("subtitle")
        );
        assert_eq!(
            stage.lower.root.get_attribute("aria-hidden").as_deref(),
            Some("false")
        );
        assert!(
            stage
                .root
                .query_selector(".tv-bubble[data-line-id=\"42\"]")
                .unwrap()
                .is_none()
        );
        finish_stage(parent, stage);
    }

    #[test]
    fn late_same_bitmap_geometry_does_not_create_dialogue_bubbles() {
        use super::super::live_stage::{BubbleKind, LineBubble, StageMode};
        use crate::{
            plate_layout::Side,
            presentation::{ActorSlot, PlateActor, PlateGeometry, SceneFamily},
        };
        let (parent, stage) = mounted_stage();
        seed_active_plate(&stage);
        let view = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    journey: Some(rpc::JourneyView::default()),
                    scene: Some(rpc::GameplayScene {
                        cast: vec![
                            rpc::SceneActor {
                                id: "courier".into(),
                                name: "Courier".into(),
                                speaking: true,
                                ..Default::default()
                            },
                            rpc::SceneActor {
                                id: "bandit".into(),
                                name: "Bandit".into(),
                                ..Default::default()
                            },
                        ],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        stage
            .figures
            .update(
                &crate::presentation::scene_cast(&view),
                "old-scene",
                crate::stage_depth::DEFAULT,
                SceneFamily::Exterior,
                StageMode::Scene,
            )
            .unwrap();
        let mut geometry = PlateGeometry {
            figure_masks: Vec::new(),
            bubble_side: Side::Right,
            head_x: 400,
            head_y: 300,
            head_width: 100,
            head_height: 100,
            heroes: vec![],
            actors: vec![],
        };
        stage
            .figures
            .set_plate(Some(geometry.clone()), Some((1920.0, 1080.0)))
            .unwrap();
        stage.plate.picture_size.set(Some((1920.0, 1080.0)));
        let slot = super::super::live_stage::slot_index(ActorSlot::Npc(1)).unwrap();
        let old = LineBubble {
            kind: BubbleKind::Speech,
            speaker_actor: crate::presentation::CaptionSpeaker::Unknown,
            speaker: "Bandit",
            text: "Keep away.",
            voiced: false,
        };
        stage
            .character_line(slot, 1, LineState::Opening, &old)
            .unwrap();
        stage
            .character_line(slot, 1, LineState::Lingering, &old)
            .unwrap();
        let host = stage
            .root
            .query_selector(".tv-fig-bubbles[data-actor-id=bandit]")
            .unwrap()
            .unwrap();
        assert!(host.has_attribute("hidden"));
        let before = host.get_attribute("style");
        geometry.actors.push(PlateActor {
            actor_id: "bandit".into(),
            bubble_side: Side::Left,
            head_x: 800,
            head_y: 250,
            head_width: 100,
            head_height: 100,
        });
        stage
            .set_plate(
                &PlateView {
                    state: PlateState::Ready,
                    url: Some("/gameplay/imagery/old".into()),
                    layout: Some(geometry.clone()),
                    ..Default::default()
                },
                "old-scene",
                true,
            )
            .unwrap();
        stage.plate_clock_at(10000.0).unwrap();
        stage
            .character_line(slot, 1, LineState::Lingering, &old)
            .unwrap();
        assert!(
            host.has_attribute("hidden"),
            "the retained named lane remains readable in its original place"
        );
        assert_eq!(host.get_attribute("style"), before);
        assert!(!stage.root.has_attribute("data-unplaced-line"));
        let next = LineBubble {
            kind: BubbleKind::Speech,
            speaker_actor: crate::presentation::CaptionSpeaker::Unknown,
            speaker: "Bandit",
            text: "Give me the packet.",
            voiced: false,
        };
        host.set_attribute("style", "width:200px;height:80px")
            .unwrap();
        stage
            .character_line(slot, 2, LineState::Opening, &next)
            .unwrap();
        assert!(host.has_attribute("hidden"));
        assert!(!host.has_attribute("data-anchor-fallback"));
        assert!(!host.has_attribute("data-anchor-line"));
        assert!(!stage.root.has_attribute("data-unplaced-line"));
        stage
            .character_line(slot, 2, LineState::Lingering, &next)
            .unwrap();
        stage.root.set_attribute("data-line-id", "2").unwrap();
        stage
            .root
            .set_attribute("data-plate-linger-at", "0")
            .unwrap();
        stage
            .set_plate(
                &PlateView {
                    state: PlateState::Ready,
                    url: Some("/gameplay/imagery/different-composition".into()),
                    layout: None,
                    ..Default::default()
                },
                "old-scene",
                false,
            )
            .unwrap();
        let image = &stage.plate.images[1];
        let request = stage.plate.root.get_attribute("data-request").unwrap();
        image.set_attribute("data-decoded", &request).unwrap();
        stage.plate_clock_at(20000.0).unwrap();
        assert_eq!(
            stage.root.get_attribute("data-plate-key").as_deref(),
            Some("different-composition")
        );
        assert!(
            host.has_attribute("hidden"),
            "old screen position must not attribute a retained line to a different person in the new photo"
        );
        assert!(!stage.root.has_attribute("data-unplaced-line"));
        assert!(!stage.root.has_attribute("data-plate-layout-key"));
        stage
            .figures
            .set_plate(None, Some((1920.0, 1080.0)))
            .unwrap();
        stage
            .character_line(slot, 3, LineState::Opening, &next)
            .unwrap();
        assert!(host.has_attribute("hidden"));
        assert_eq!(
            host.get_attribute("data-anchor-fallback").as_deref(),
            Some("missing-image-layout")
        );
        assert!(!stage.root.has_attribute("data-unplaced-line"));
        stage
            .character_line(slot, 2, LineState::Done, &next)
            .unwrap();
        assert!(!stage.root.has_attribute("data-unplaced-line"));
        stage.figures.set_plate(None, None).unwrap();
        host.set_attribute("hidden", "").unwrap();
        host.set_attribute(
            "style",
            "position:fixed;left:0px;top:0px;width:0px;right:auto;bottom:auto;transform:none",
        )
        .unwrap();
        stage
            .figures
            .latch_plate_geometry(Some(geometry), Some((1920.0, 1080.0)))
            .unwrap();
        assert!(host.has_attribute("hidden"));
        assert!(
            !host
                .get_attribute("style")
                .unwrap_or_default()
                .contains("width:0px"),
            "hidden zero bounds cannot become a fixed one-glyph column"
        );
        stage
            .character_line(slot, 3, LineState::Lingering, &next)
            .unwrap();
        assert!(host.has_attribute("hidden"));
        host.set_attribute("style", "width:200px;height:80px")
            .unwrap();
        stage
            .character_line(slot, 4, LineState::Opening, &next)
            .unwrap();
        assert!(host.has_attribute("hidden"));
        assert!(
            !host
                .get_attribute("style")
                .unwrap_or_default()
                .contains("width:0px")
        );
        finish_stage(parent, stage);
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
    fn plate_loading_status_is_polite_and_layout_addressable() {
        let (parent, stage) = mounted_stage();
        let loading = stage
            .root
            .query_selector(".tv-plate-loading")
            .unwrap()
            .unwrap();
        assert_eq!(loading.get_attribute("role").as_deref(), Some("status"));
        assert_eq!(
            loading.get_attribute("aria-live").as_deref(),
            Some("polite")
        );
        assert_eq!(
            loading.get_attribute("data-layout-id").as_deref(),
            Some("plate-loading")
        );
        assert_eq!(loading.text_content().as_deref(), Some("Updating scene…"));
        finish_stage(parent, stage);
    }

    #[test]
    fn narration_at_a_new_spot_keeps_the_same_places_plate_awaiting_its_successor() {
        let (parent, stage) = mounted_stage();
        seed_active_plate(&stage);
        stage.same_place.set(true);
        let plate = PlateView {
            state: PlateState::Ready,
            url: Some("/gameplay/imagery/next-plate".to_owned()),
            layout: None,
            ..Default::default()
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
        assert!(!stage.root.has_attribute("data-plate-loading"));
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
    fn idle_clock_keeps_decoded_plate_until_linger_then_retires_lost_transition() {
        let (parent, stage) = mounted_stage();
        seed_active_plate(&stage);
        stage
            .lower_third(
                7,
                crate::presentation::CaptionSpeaker::Unknown,
                "Courier",
                "Keep this line steady.",
                LineState::Speaking,
            )
            .unwrap();
        let plate = PlateView {
            state: PlateState::Ready,
            url: Some("/gameplay/imagery/clock-next".to_owned()),
            ..Default::default()
        };
        stage.apply_plate(&plate, "next-scene").unwrap();
        let request = stage.plate.root.get_attribute("data-request").unwrap();
        stage.plate.images[1]
            .set_attribute("data-decoded", &request)
            .unwrap();
        // Use the scheduler's actual work predicate, with an idle conductor, rather than
        // supplying another view or speaker update to make the image activate.
        let busy = || super::super::live_motion::clock_needs_work(false, Some(&stage));
        assert!(busy());
        stage.plate_clock_at(1000.0).unwrap();
        assert_eq!(
            stage.plate.root.get_attribute("data-active").as_deref(),
            Some("0")
        );
        stage
            .lower_third(
                7,
                crate::presentation::CaptionSpeaker::Unknown,
                "Courier",
                "Keep this line steady.",
                LineState::Lingering,
            )
            .unwrap();
        stage
            .root
            .set_attribute("data-plate-linger-at", "1000")
            .unwrap();
        stage.plate_clock_at(3499.0).unwrap();
        assert!(busy());
        assert_eq!(
            stage.plate.root.get_attribute("data-active").as_deref(),
            Some("0")
        );
        stage.plate_clock_at(3500.0).unwrap();
        assert_eq!(
            stage.plate.root.get_attribute("data-active").as_deref(),
            Some("1")
        );
        assert!(busy());
        stage.plate_clock_at(4500.0).unwrap();
        assert!(stage.plate.root.has_attribute("data-previous"));
        stage.plate_clock_at(4600.0).unwrap();
        assert!(!stage.plate.root.has_attribute("data-previous"));
        assert!(!busy());
        assert_eq!(
            stage.lower.text.text_content().as_deref(),
            Some("Keep this line steady.")
        );
        finish_stage(parent, stage);
    }

    #[test]
    fn decoded_plate_waits_for_the_current_line_then_retains_the_outgoing_picture() {
        let (parent, stage) = mounted_stage();
        seed_active_plate(&stage);
        stage
            .lower_third(
                7,
                crate::presentation::CaptionSpeaker::Unknown,
                "Courier",
                "Keep this line steady.",
                LineState::Speaking,
            )
            .unwrap();
        let plate = PlateView {
            state: PlateState::Ready,
            url: Some("/gameplay/imagery/decoded-next".to_owned()),
            ..Default::default()
        };
        stage.apply_plate(&plate, "next-scene").unwrap();
        let image = &stage.plate.images[1];
        let request = stage.plate.root.get_attribute("data-request").unwrap();
        image.set_attribute("data-decoded", &request).unwrap();
        stage.set_speaker("character").unwrap();
        assert_eq!(
            stage.plate.root.get_attribute("data-active").as_deref(),
            Some("0")
        );
        stage
            .lower_third(
                7,
                crate::presentation::CaptionSpeaker::Unknown,
                "Courier",
                "Keep this line steady.",
                LineState::Done,
            )
            .unwrap();
        stage.set_speaker("").unwrap();
        assert_eq!(
            stage.plate.root.get_attribute("data-active").as_deref(),
            Some("1")
        );
        assert_eq!(
            stage.plate.root.get_attribute("data-previous").as_deref(),
            Some("0")
        );
        assert_eq!(
            stage.lower.text.text_content().as_deref(),
            Some("Keep this line steady.")
        );
        finish_stage(parent, stage);
    }

    #[test]
    fn pending_successor_invalidates_the_previous_scenes_decode() {
        let (parent, stage) = mounted_stage();
        seed_active_plate(&stage);
        let plate = PlateView {
            state: PlateState::Ready,
            url: Some("/gameplay/imagery/old-scene".to_owned()),
            ..Default::default()
        };
        stage.apply_plate(&plate, "old-scene").unwrap();
        let image = &stage.plate.images[1];
        let request = stage.plate.root.get_attribute("data-request").unwrap();
        stage
            .apply_plate(
                &PlateView {
                    state: PlateState::Pending,
                    ..Default::default()
                },
                "successor",
            )
            .unwrap();
        image.set_attribute("data-decoded", &request).unwrap();
        stage.set_speaker("").unwrap();
        assert!(!stage.plate.root.has_attribute("data-active"));
        assert!(!stage.plate.root.has_attribute("data-applied-request"));
        finish_stage(parent, stage);
    }

    #[test]
    fn late_join_context_ignores_old_terminal_lines_and_yields_to_current_narration() {
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
        assert_eq!(stage.narration.text_content().as_deref(), Some(""));
        stage
            .narrator_line(2, LineState::Opening, "The current scene speaks.")
            .unwrap();
        assert!(
            stage
                .root
                .query_selector(".tv-scene-context")
                .unwrap()
                .is_none()
        );
        stage
            .set_scene_context("A letter lies on the table.")
            .unwrap();
        assert!(
            stage
                .root
                .query_selector(".tv-scene-context")
                .unwrap()
                .is_none()
        );
        finish_stage(parent, stage);
    }

    #[test]
    fn same_place_scene_plate_keeps_the_current_plate_visible_until_replacement_is_ready() {
        let (parent, stage) = mounted_stage();
        seed_active_plate(&stage);
        stage.same_place.set(true);
        let replacement = PlateView {
            state: PlateState::Ready,
            url: Some("/gameplay/imagery/new".to_owned()),
            layout: None,
            ..Default::default()
        };

        stage.set_plate(&replacement, "new-scene", true).unwrap();

        assert_eq!(
            stage.plate.root.get_attribute("data-active").as_deref(),
            Some("0")
        );
        assert!(stage.root.has_attribute("data-plate-loading"));
        assert_eq!(
            stage.plate.root.get_attribute("data-want-i").as_deref(),
            Some("1")
        );
        assert!(stage.root.has_attribute("data-plate-loaded"));
        finish_stage(parent, stage);
    }

    #[test]
    fn pending_new_scene_keeps_the_previous_plate_under_the_new_scene_figures() {
        let (parent, stage) = mounted_stage();
        stage.root.set_attribute("data-tv-mode", "scene").unwrap();
        seed_active_plate(&stage);
        stage.root.set_attribute("data-plate-key", "old").unwrap();
        stage
            .plate
            .root
            .set_attribute("data-transition", "0")
            .unwrap();
        stage.root.set_attribute("data-mural", "ready").unwrap();
        stage
            .root
            .set_attribute("data-scene-key", "new-scene")
            .unwrap();
        let pending = PlateView {
            state: PlateState::Pending,
            url: None,
            layout: None,
            update_pending: true,
            ..Default::default()
        };

        stage.set_plate(&pending, "new-scene", true).unwrap();

        assert_eq!(
            stage.plate.root.get_attribute("data-active").as_deref(),
            Some("0")
        );
        assert!(stage.root.has_attribute("data-plate-loaded"));
        assert_eq!(
            stage.root.get_attribute("data-plate-key").as_deref(),
            Some("old")
        );
        assert!(stage.root.has_attribute("data-place-plate-retained"));
        assert_eq!(
            stage.root.get_attribute("data-shot").as_deref(),
            Some("stage")
        );
        assert!(!stage.plate.root.has_attribute("data-transition"));
        assert_eq!(
            stage.root.get_attribute("data-plate-state").as_deref(),
            Some("pending")
        );
        assert!(stage.root.has_attribute("data-plate-loading"));
        assert!(stage.root.has_attribute("data-place-plate-retained"));
        let ready = PlateView {
            state: PlateState::Ready,
            url: Some("/gameplay/imagery/old".to_owned()),
            ..Default::default()
        };
        stage.set_plate(&ready, "new-scene", true).unwrap();
        assert!(stage.root.has_attribute("data-plate-loading"));
        finish_stage(parent, stage);
    }

    #[test]
    fn photographic_establishing_survives_graph_arrival_until_the_plate_is_activated() {
        let (parent, stage) = mounted_stage();
        let document = stage.root.owner_document().unwrap();
        let style = document.create_element("style").unwrap();
        style.set_text_content(Some(&format!(
            "{}\n{}\n.tv-stage *{{transition:none!important;animation:none!important}}",
            include_str!("live_stage.css"),
            include_str!("live_plate.css")
        )));
        parent.append_child(&style).unwrap();
        stage.root.set_attribute("data-tv-mode", "scene").unwrap();
        stage
            .root
            .set_attribute("data-battle-state", "absent")
            .unwrap();
        stage.root.set_attribute("data-stage-graph", "").unwrap();
        stage
            .root
            .set_attribute("data-shot", "establishing")
            .unwrap();
        let window = web_sys::window().unwrap();
        let computed = js_sys::Reflect::get(&window, &"getComputedStyle".into())
            .unwrap()
            .dyn_into::<js_sys::Function>()
            .unwrap();
        let graph = stage
            .root
            .query_selector(".tv-scene-graph")
            .unwrap()
            .unwrap();
        let art = stage.root.query_selector(".tv-art").unwrap().unwrap();
        for state in ["pending", "ready"] {
            stage.root.set_attribute("data-plate-state", state).unwrap();
            let css = computed.call1(&window, &graph).unwrap();
            assert_eq!(
                js_sys::Reflect::get(&css, &"opacity".into())
                    .unwrap()
                    .as_string()
                    .as_deref(),
                Some("0")
            );
            let css = computed.call1(&window, &art).unwrap();
            assert_ne!(
                js_sys::Reflect::get(&css, &"visibility".into())
                    .unwrap()
                    .as_string()
                    .as_deref(),
                Some("hidden")
            );
        }
        stage.root.set_attribute("data-plate-loaded", "").unwrap();
        stage.root.set_attribute("data-shot", "plate").unwrap();
        let css = computed.call1(&window, &stage.plate.root).unwrap();
        assert_eq!(
            js_sys::Reflect::get(&css, &"opacity".into())
                .unwrap()
                .as_string()
                .as_deref(),
            Some("1")
        );
        assert_eq!(
            js_sys::Reflect::get(&css, &"visibility".into())
                .unwrap()
                .as_string()
                .as_deref(),
            Some("visible")
        );
        finish_stage(parent, stage);
    }

    #[test]
    fn absent_metadata_during_same_scene_refresh_does_not_clear_the_loaded_plate() {
        let (parent, stage) = mounted_stage();
        seed_active_plate(&stage);
        for state in [
            PlateState::None,
            PlateState::Unavailable,
            PlateState::Pending,
            PlateState::Failed,
        ] {
            stage
                .set_plate(
                    &PlateView {
                        state,
                        ..Default::default()
                    },
                    "old-scene",
                    false,
                )
                .unwrap();
            assert!(stage.root.has_attribute("data-plate-loaded"));
            assert_eq!(
                stage.root.get_attribute("data-shot").as_deref(),
                Some("plate")
            );
            assert_eq!(
                stage.plate.root.get_attribute("data-active").as_deref(),
                Some("0")
            );
        }
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
        stage.root.set_attribute("data-plate-loading", "").unwrap();

        stage.plate.images[1]
            .dispatch_event(&web_sys::Event::new("error").unwrap())
            .unwrap();

        assert!(!stage.root.has_attribute("data-plate-loading"));
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
        assert!(stage.root.has_attribute("data-sound-opted-out"));
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
                audio_url: String::new(),
                audio_part_urls: Vec::new(),
                role: rpc::TableRole::Narrator as i32,
                kind: rpc::TableKind::Action as i32,
                speaker: "Roll".to_owned(),
                text: "Brakka searches the room: Wisdom (Perception) 16 + 3 = 19.".to_owned(),
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
        assert_eq!(shown.intention, "Brakka searches the room");
        assert_eq!(shown.ability_skill, "Wisdom (Perception)");
        assert_eq!(shown.faces, [(16, true), (4, false)]);
        assert_eq!(shown.modifier, 3);
        assert_eq!(shown.total, 19);
        assert!(shown.succeeded);
        assert_eq!(shown.finding, "The bronze ledger bears a seal. More text.");
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
                    audio_url: String::new(),
                    audio_part_urls: Vec::new(),
                    role: rpc::TableRole::Hero as i32,
                    kind: rpc::TableKind::Action as i32,
                    speaker: "Brakka".to_owned(),
                    ..Default::default()
                },
                rpc::TableLine {
                    audio_url: String::new(),
                    audio_part_urls: Vec::new(),
                    role: rpc::TableRole::Narrator as i32,
                    hero_index: 1,
                    kind: rpc::TableKind::Action as i32,
                    speaker: "Roll".to_owned(),
                    text: "Sigrun studies the seal: Wisdom (Perception) 4 - 1 = 3.".to_owned(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let shown = check_display(&check, &journey, "Nothing new comes to light.")
            .expect("a complete check has a display projection");
        assert_eq!(shown.hero_name, "Sigrun");
        assert_eq!(shown.intention, "Sigrun studies the seal");
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
    fn roll_intention_keeps_public_action_clause_and_full_text_without_separator() {
        assert_eq!(
            roll_intention("Brakka searches the room: Wisdom (Perception) 13 + 1 = 14."),
            "Brakka searches the room"
        );
        assert_eq!(
            roll_intention("Inspect the rune — no metadata separator."),
            "Inspect the rune — no metadata separator."
        );
        assert_eq!(
            roll_intention("Brakka checks the ward: source: public"),
            "Brakka checks the ward: source"
        );
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
    fn roll_and_figureless_reply_retire_old_narration_without_resurrecting_it() {
        use super::super::live_stage::{BubbleKind, CheckDisplay, LineBubble};
        let (parent, stage) = mounted_stage();
        stage
            .narrator_line(21, LineState::Opening, "Finn has no coin you could lift.")
            .unwrap();
        stage
            .narrator_line(21, LineState::Done, "Finn has no coin you could lift.")
            .unwrap();
        stage
            .begin_check(
                &CheckDisplay {
                    sequence: 22,
                    hero_name: "Sylvara".into(),
                    intention: "Ask about the wager".into(),
                    ability_skill: "Charisma".into(),
                    faces: vec![(12, true)],
                    roll_mode: 0,
                    modifier: 0,
                    total: 12,
                    succeeded: true,
                    finding: "Dain answers.".into(),
                },
                false,
            )
            .unwrap();
        stage.finish_check(22).unwrap();
        assert_eq!(stage.narration.text_content().as_deref(), Some(""));
        stage
            .narrator_line(21, LineState::Done, "Finn has no coin you could lift.")
            .unwrap();
        assert_eq!(stage.narration.text_content().as_deref(), Some(""));
        stage
            .character_line(
                usize::MAX,
                24,
                LineState::Opening,
                &LineBubble {
                    kind: BubbleKind::Speech,
                    speaker_actor: crate::presentation::CaptionSpeaker::Unknown,
                    speaker: "Dain Rusk",
                    text: "A widow's a grim wager.",
                    voiced: true,
                },
            )
            .unwrap();
        assert_eq!(
            stage.lower.name.text_content().as_deref(),
            Some("Dain Rusk")
        );
        assert_eq!(
            stage.lower.text.text_content().as_deref(),
            Some("A widow's a grim wager.")
        );
        stage.set_speaker("").unwrap();
        assert_eq!(stage.narration.text_content().as_deref(), Some(""));
        stage
            .narrator_line(25, LineState::Opening, "The newest narration.")
            .unwrap();
        stage
            .narrator_line(21, LineState::Closing, "Finn has no coin you could lift.")
            .unwrap();
        assert_eq!(
            stage.narration.text_content().as_deref(),
            Some("The newest narration.")
        );
        finish_stage(parent, stage);
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

/// Executes the real WASM DOM boundary in the owned, provider-free browser fixture.
#[cfg(any(test, feature = "public-scene-delivery-fixture"))]
#[cfg_attr(
    feature = "public-scene-delivery-fixture",
    wasm_bindgen::prelude::wasm_bindgen
)]
pub fn party_mural_dom_check() -> Result<String, JsValue> {
    let document = web_sys::window()
        .ok_or_else(|| JsValue::from_str("window absent"))?
        .document()
        .ok_or_else(|| JsValue::from_str("document absent"))?;
    let parent = document.create_element("section")?;
    document
        .body()
        .ok_or_else(|| JsValue::from_str("body absent"))?
        .append_child(&parent)?;
    let result = (|| {
        let stage = TvStage::mount(&document, &parent)?;
        stage.root.set_attribute("data-tv-mode", "scene")?;
        stage.update_mural_wire("{}")?;
        if stage.root.has_attribute("data-arrival-mural") || stage.mural_art.has_attribute("src") {
            return Err(JsValue::from_str(
                "missing mural displaced the room photo fallback",
            ));
        }
        let url = format!("/gameplay/imagery/{}", "a".repeat(64));
        let contents = serde_json::json!({"party_mural":{"state":"ready","url":url}}).to_string();
        stage.update_mural_wire(&contents)?;
        stage.update_mural_wire(&contents)?;
        let mural = stage
            .root
            .query_selector(":scope > .tv-mural")?
            .ok_or_else(|| JsValue::from_str("direct mural layer absent"))?;
        if mural.get_attribute("aria-hidden").as_deref() != Some("true")
            || stage.mural_art.get_attribute("alt").as_deref() != Some("")
            || stage.mural_art.get_attribute("src").as_deref() != Some(&url)
            || stage.root.get_attribute("data-mural").as_deref() != Some("ready")
        {
            return Err(JsValue::from_str("ready mural DOM mismatch"));
        }
        if stage.root.query_selector_all(".tv-mural")?.length() != 1 {
            return Err(JsValue::from_str("mural duplicated across repeated views"));
        }
        for contents in [
            "{\"party_mural\":{\"state\":\"pending\",\"url\":\"\"}}",
            "{\"party_mural\":{\"state\":\"failed\",\"url\":\"\"}}",
            "{\"party_mural\":{\"state\":\"ready\",\"url\":\"https://elsewhere/image\"}}",
            "{}",
        ] {
            stage.update_mural_wire(contents)?;
            if stage.root.get_attribute("data-mural").as_deref() != Some("ready")
                || stage.mural_art.get_attribute("src").as_deref() != Some(&url)
            {
                return Err(JsValue::from_str(
                    "cached session mural lost on scene metadata update",
                ));
            }
        }
        stage.update_mural_wire(&contents)?;
        stage.dispose()?;
        Ok("party mural WASM DOM: ready, repeated view, pending, failed, invalid URL, missing member and ready recovery passed".to_owned())
    })();
    parent.remove();
    result
}

#[cfg(test)]
mod mural_dom_tests {
    #[test]
    fn party_mural_sets_one_image_and_stage_attribute_from_validated_server_contents() {
        super::party_mural_dom_check().unwrap();
    }
}

/// Provider-free browser check of the place-change boundary, including a lost dissolve event.
#[cfg(any(test, feature = "public-scene-delivery-fixture"))]
#[cfg_attr(
    feature = "public-scene-delivery-fixture",
    wasm_bindgen::prelude::wasm_bindgen
)]
pub fn free_roam_arrival_dom_check() -> Result<String, JsValue> {
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("document absent"))?;
    let parent = document.create_element("section")?;
    document
        .body()
        .ok_or_else(|| JsValue::from_str("body absent"))?
        .append_child(&parent)?;
    let result = (|| {
        let stage = TvStage::mount(&document, &parent)?;
        stage.root.set_attribute("data-tv-mode", "scene")?;
        stage.root.set_attribute("data-shot", "plate")?;
        stage.root.set_attribute("data-scene-key", "gate")?;
        stage.root.set_attribute("data-plate-state", "ready")?;
        stage.root.set_attribute("data-plate-key", "gate-picture")?;
        stage.root.set_attribute("data-plate-loaded", "")?;
        stage.plate.root.set_attribute("data-active", "0")?;
        *stage.plate.scene.borrow_mut() = "gate".to_owned();
        *stage.plate.seen.borrow_mut() = "gate".to_owned();
        let pending = PlateView {
            state: PlateState::Pending,
            ..Default::default()
        };
        stage.set_plate(&pending, "gate", true)?;
        if !stage.root.has_attribute("data-plate-key")
            || !stage.root.has_attribute("data-plate-loaded")
        {
            return Err(JsValue::from_str("same-place update lost its ready plate"));
        }
        stage.update_mural_wire(&serde_json::json!({"party_mural":{"state":"ready","url":format!("/gameplay/imagery/{}", "a".repeat(64))}}).to_string())?;
        stage.same_place.set(true);
        stage.root.set_attribute("data-scene-key", "gate-spot")?;
        stage.set_plate(&pending, "gate-spot", true)?;
        if !stage.root.has_attribute("data-plate-loaded")
            || stage.root.has_attribute("data-arrival-mural")
            || stage.root.get_attribute("data-shot").as_deref() != Some("plate")
        {
            return Err(JsValue::from_str("spot move replaced the current plate"));
        }
        stage.same_place.set(false);
        stage.update_mural_wire("{}")?;
        stage.plate.root.set_attribute("data-transition", "0")?;
        stage.root.set_attribute("data-scene-key", "market")?;
        stage.set_plate(&pending, "market", true)?;
        if stage.root.has_attribute("data-plate-key")
            || stage.root.has_attribute("data-plate-loaded")
            || stage.plate.root.has_attribute("data-active")
            || stage.plate.root.has_attribute("data-transition")
            || !stage.root.has_attribute("data-plate-loading")
            || !stage.root.has_attribute("data-arrival-mural")
        {
            return Err(JsValue::from_str(
                "new-place pending plate retained stale art or hid mural fallback",
            ));
        }
        let ready = PlateView {
            state: PlateState::Ready,
            url: Some("/gameplay/imagery/market-picture".to_owned()),
            ..Default::default()
        };
        stage.set_plate(&ready, "market", true)?;
        if !stage.root.has_attribute("data-arrival-mural") {
            return Err(JsValue::from_str("mural hidden before plate decode"));
        }
        let index = stage
            .plate
            .root
            .get_attribute("data-want-i")
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| JsValue::from_str("plate frame absent"))?;
        let image = stage
            .plate
            .images
            .get(index)
            .ok_or_else(|| JsValue::from_str("plate image absent"))?;
        let request = stage
            .plate
            .root
            .get_attribute("data-request")
            .ok_or_else(|| JsValue::from_str("plate request absent"))?;
        image.set_attribute("data-decoded", &request)?;
        stage.narration_over()?;
        activate_plate_request_at(
            &stage.root,
            &stage.plate.root,
            image,
            &Rc::downgrade(&stage.figures),
            &stage.plate.pending_layout,
            &stage.plate.picture_size,
            20_000.0,
        );
        if !stage.root.has_attribute("data-plate-loaded")
            || stage.root.has_attribute("data-arrival-mural")
            || stage.root.get_attribute("data-shot").as_deref() != Some("plate")
        {
            return Err(JsValue::from_str("decoded plate did not replace mural"));
        }
        stage.root.set_attribute("data-scene-key", "inn")?;
        let inn = PlateView {
            url: Some("/gameplay/imagery/inn-picture".to_owned()),
            ..ready
        };
        stage.set_plate(&inn, "inn", true)?;
        if stage.root.has_attribute("data-plate-loaded")
            || !stage.root.has_attribute("data-arrival-mural")
            || stage.plate.root.has_attribute("data-active")
        {
            return Err(JsValue::from_str(
                "arrival retained another place while decoding a ready plate",
            ));
        }
        stage.dispose()?;
        Ok("free-roam arrival WASM DOM: spot retention, cached mural, pending and decoding arrivals, plate handoff passed".to_owned())
    })();
    parent.remove();
    result
}

#[cfg(test)]
mod arrival_dom_tests {
    #[test]
    fn free_roam_arrival_clears_stale_plate_and_exposes_party_mural() {
        super::free_roam_arrival_dom_check().unwrap();
    }
}
