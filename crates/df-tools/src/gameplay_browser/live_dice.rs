//! The visible d20 roll over the fight, played from real video clips.
//!
//! For every d20 test the server reports, the TV plays one of the clips from the
//! `dice-rolls.json` manifest over the scene: the die enters, tumbles and comes to rest,
//! and at the clip's own rest time the server's number appears on the upward face,
//! positioned, scaled and rotated from the manifest's face coordinates. Advantage or
//! disadvantage plays two different clips side by side, the counted one marked.
//!
//! The browser decides nothing: faces come from [`dice_roll`](crate::dice_roll) (the
//! server's kept face and, for two dice, its discarded draw), clip choice is a hash of the
//! server's own numbers so a replay shows the same clip, and the number shown is only ever
//! the server's. Clips are fetched once when a fight starts and reused; a roll whose clip
//! is not ready shows the roll card and never waits on the network.
//!
//! Transparency, decided by testing in the page at start-up, never by browser name: the
//! WebM plays where the browser shows its alpha channel, otherwise the key-colour MP4 is
//! drawn through a canvas with the key removed, otherwise only the resting frame shows
//! (animated, so a roll is still visible). This machine's Chrome takes the WebM path: its
//! probe reads 229k clear and 31k solid pixels of 262k. The still resting frame seen on the
//! TV was never a probe failure: the pane reports `prefers-reduced-motion: reduce`, and the
//! die used to honour it by showing the resting frame only. The dice are the game's one
//! indispensable visual on a shared screen, so the clip now plays whatever the machine's
//! motion preference says; the surrounding decoration still follows it.
//!
//! The canvas calls are untyped on purpose: the canvas web-sys features would need another
//! worker's `Cargo.toml`, so this one small module calls the half-dozen DOM methods through
//! `js_sys::Reflect` instead.
//!
//! Where the dice live: the 3D battlefield paints over the whole stage (see the stacking
//! order in `live_tv.css`), so while it is mounted the dice are hosted inside its
//! container, above its canvas and below its HUD; on the flat stage they stay on the stage
//! root, below its cards.
use super::*;
use crate::dice_roll::{DiceClip, DiceManifest, DicePlay, Natural, RollMode};
use crate::roll_card::RollCard;
use crate::stage_machines::LineState;
use df_observe::presentation::{ComponentPhase, PresentationComponent};
use gloo_timers::future::TimeoutFuture;
use js_sys::{Array, Function, Reflect, Uint8ClampedArray};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Blob, BlobPropertyBag, Response, Url};

/// The manifest the server routes, extensionless like every other extra asset.
const MANIFEST_URL: &str = "/assets/ui/generated/dice/dice-rolls";
/// Clips aim under 400 KB; anything absurd is refused instead of cached.
const MAXIMUM_CLIP_BYTES: u32 = 2 * 1024 * 1024;
const MAXIMUM_MANIFEST_BYTES: u32 = 64 * 1024;
/// The clip square the keying canvas works at.
const CLIP_SIZE: u32 = 512;
/// Key match distance squared: inside it the pixel is gone, past the feather it stays.
const KEY_FULL_OUT: i32 = 90 * 90;
const KEY_FEATHER_OUT: i32 = 140 * 140;
/// How long the transparency probe waits for a clip event before giving up on it.
const PROBE_TIMEOUT_MS: i32 = 4_000;

/// Which playback the start-up probe settled on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DicePath {
    Undecided,
    WebM,
    Key,
    CardOnly,
}

struct ReadyClip {
    clip: DiceClip,
    webm_url: String,
    key_url: String,
    rest_url: String,
}

struct Library {
    clips: Vec<ReadyClip>,
    key: [u8; 3],
    path: DicePath,
    last: Option<usize>,
    loading: bool,
    timeouts: Vec<i32>,
    held: Vec<Closure<dyn FnMut()>>,
    listeners: Vec<Closure<dyn FnMut(web_sys::Event)>>,
    painters: Vec<Closure<dyn FnMut(f64)>>,
    manifest_warned: bool,
}

thread_local! {
    static LIBRARY: RefCell<Library> = RefCell::new(Library {
        clips: Vec::new(),
        key: [255, 0, 255],
        path: DicePath::Undecided,
        last: None,
        loading: false,
        timeouts: Vec::new(),
        held: Vec::new(),
        listeners: Vec::new(),
        painters: Vec::new(),
        manifest_warned: false,
    });
}

/// Fetches the manifest and the clips once per page; later calls are no-ops. Never blocks
/// a roll: anything missing or refusing simply leaves the card-only fallback in place.
pub(super) fn preload() {
    let started = LIBRARY.with(|library| {
        let mut library = library.borrow_mut();
        if library.loading || !library.clips.is_empty() {
            return false;
        }
        library.loading = true;
        true
    });
    if started {
        spawn_local(load_library());
    }
}

fn warn_manifest_unavailable() {
    let should_warn = LIBRARY.with(|library| {
        let mut library = library.borrow_mut();
        if library.manifest_warned {
            false
        } else {
            library.manifest_warned = true;
            true
        }
    });
    if should_warn {
        web_sys::console::warn_1(&JsValue::from_str(
            "Dice manifest unavailable; using still dice images.",
        ));
    }
}

async fn load_library() {
    let manifest = fetch_text(MANIFEST_URL, MAXIMUM_MANIFEST_BYTES).await;
    let manifest: Option<DiceManifest> =
        manifest.and_then(|text| crate::dice_roll::parse_manifest(&text));
    let Some(manifest) = manifest else {
        warn_manifest_unavailable();
        finish_loading(Vec::new(), [255, 0, 255], DicePath::CardOnly);
        return;
    };
    let mut clips = Vec::new();
    for clip in &manifest.clips {
        let (Some(webm), Some(key), Some(rest)) = (
            fetch_blob(&crate::dice_roll::clip_url(&clip.webm), "video/webm").await,
            fetch_blob(&crate::dice_roll::clip_url(&clip.key_mp4), "video/mp4").await,
            fetch_blob(&crate::dice_roll::clip_url(&clip.rest_image), "image/webp").await,
        ) else {
            continue;
        };
        clips.push(ReadyClip {
            clip: clip.clone(),
            webm_url: webm,
            key_url: key,
            rest_url: rest,
        });
    }
    if clips.is_empty() {
        finish_loading(Vec::new(), [255, 0, 255], DicePath::CardOnly);
        return;
    }
    let key = parse_key(&manifest.key_color);
    let path = decide_path(&clips).await;
    finish_loading(clips, key, path);
}

fn finish_loading(clips: Vec<ReadyClip>, key: [u8; 3], path: DicePath) {
    LIBRARY.with(|library| {
        let mut library = library.borrow_mut();
        library.clips = clips;
        library.key = key;
        library.path = path;
    });
    // Diagnostic trail on the document element: which playback the probe settled on.
    if let Some(root) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
    {
        let _marked = root.set_attribute(
            "data-dice-path",
            match path {
                DicePath::Undecided => "undecided",
                DicePath::WebM => "webm",
                DicePath::Key => "key",
                DicePath::CardOnly => "card",
            },
        );
    }
}

async fn fetch_text(url: &str, maximum_bytes: u32) -> Option<String> {
    let window = web_sys::window()?;
    let response: Response = JsFuture::from(window.fetch_with_str(url))
        .await
        .ok()?
        .dyn_into()
        .ok()?;
    if response.status() != 200 {
        return None;
    }
    let text = JsFuture::from(response.text().ok()?).await.ok()?;
    let text = text.as_string()?;
    if text.is_empty() || text.len() as u32 > maximum_bytes {
        return None;
    }
    Some(text)
}

async fn fetch_blob(url: &str, mime: &str) -> Option<String> {
    let window = web_sys::window()?;
    let response: Response = JsFuture::from(window.fetch_with_str(url))
        .await
        .ok()?
        .dyn_into()
        .ok()?;
    if response.status() != 200 {
        return None;
    }
    let buffer = JsFuture::from(response.array_buffer().ok()?).await.ok()?;
    let bytes = js_sys::Uint8Array::new(&buffer);
    if bytes.length() == 0 || bytes.length() > MAXIMUM_CLIP_BYTES {
        return None;
    }
    let options = BlobPropertyBag::new();
    options.set_type(mime);
    let blob = Blob::new_with_u8_array_sequence_and_options(&Array::of1(&bytes), &options).ok()?;
    Url::create_object_url_with_blob(&blob).ok()
}

/// Decides the playback by testing in this page: a WebM frame with both transparent and
/// opaque pixels plays the alpha path, a key-colour MP4 frame that decodes plays the key
/// path, otherwise the roll card stands alone.
async fn decide_path(clips: &[ReadyClip]) -> DicePath {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return DicePath::CardOnly;
    };
    if probe_alpha(&document, &clips[0].webm_url).await {
        return DicePath::WebM;
    }
    if probe_plays(&document, &clips[0].key_url).await {
        return DicePath::Key;
    }
    DicePath::CardOnly
}

/// Resolves `true` on the first of the element's events named `ok`, `false` on error or
/// after the probe timeout. The one-shot listeners are intentionally forgotten: the probe
/// runs once per page and both closures drop with their element.
fn once(element: &Element, ok: &str) -> js_sys::Promise {
    js_sys::Promise::new(&mut |resolve: Function, _reject: Function| {
        let settled = Rc::new(RefCell::new(false));
        let resolve_failed = resolve.clone();
        let resolve_ok = settled.clone();
        let done = Closure::wrap(Box::new(move |_: web_sys::Event| {
            if !*resolve_ok.borrow() {
                *resolve_ok.borrow_mut() = true;
                let _resolved = resolve.call1(&JsValue::NULL, &JsValue::TRUE);
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        let resolve_err = settled.clone();
        let failed = Closure::wrap(Box::new(move |_: web_sys::Event| {
            if !*resolve_err.borrow() {
                *resolve_err.borrow_mut() = true;
                let _resolved = resolve_failed.call1(&JsValue::NULL, &JsValue::FALSE);
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        let _ = element.add_event_listener_with_callback(ok, done.as_ref().unchecked_ref());
        let _ = element.add_event_listener_with_callback("error", failed.as_ref().unchecked_ref());
        done.forget();
        failed.forget();
    })
}

/// Races a probe promise against a timeout that resolves `false`.
fn race_timeout(promise: js_sys::Promise) -> js_sys::Promise {
    let timer = js_sys::Promise::new(&mut |resolve: Function, _: Function| {
        let tick = Closure::wrap(Box::new(move || {
            let _resolved = resolve.call1(&JsValue::NULL, &JsValue::FALSE);
        }) as Box<dyn FnMut()>);
        if let Some(window) = web_sys::window() {
            let _id = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                tick.as_ref().unchecked_ref(),
                PROBE_TIMEOUT_MS,
            );
        }
        tick.forget();
    });
    js_sys::Promise::race(&Array::of2(&promise, &timer))
}

fn probe_video(document: &Document, url: &str) -> Result<web_sys::HtmlVideoElement, JsValue> {
    let video: web_sys::HtmlVideoElement = document.create_element("video")?.dyn_into()?;
    video.set_muted(true);
    video.set_attribute("muted", "")?;
    video.set_attribute("playsinline", "")?;
    video.set_attribute("preload", "auto")?;
    video.set_src(url);
    Ok(video)
}

/// A WebM frame counts as transparent when it carries both clear background and solid
/// die: over one percent of pixels at each end of the alpha range.
async fn probe_alpha(document: &Document, url: &str) -> bool {
    let Ok(video) = probe_video(document, url) else {
        return false;
    };
    if JsFuture::from(race_timeout(once(&video, "loadeddata")))
        .await
        .map(|settled| settled == JsValue::TRUE)
        .unwrap_or(false)
    {
        video.set_current_time(0.6);
    } else {
        return false;
    }
    if !JsFuture::from(race_timeout(once(&video, "seeked")))
        .await
        .map(|settled| settled == JsValue::TRUE)
        .unwrap_or(false)
    {
        return false;
    }
    let pixels = match draw_pixels(document, &video) {
        Ok(pixels) => pixels,
        Err(_) => return false,
    };
    let mut clear = 0usize;
    let mut solid = 0usize;
    for alpha in pixels.iter().skip(3).step_by(4) {
        if *alpha < 16 {
            clear += 1;
        } else if *alpha > 240 {
            solid += 1;
        }
    }
    let total = pixels.len() / 4;
    clear > total / 100 && solid > total / 100
}

/// The key MP4 only needs to decode: a frame carrying both the key colour and other
/// colours plays the canvas path.
async fn probe_plays(document: &Document, url: &str) -> bool {
    let Ok(video) = probe_video(document, url) else {
        return false;
    };
    if !JsFuture::from(race_timeout(once(&video, "loadeddata")))
        .await
        .map(|settled| settled == JsValue::TRUE)
        .unwrap_or(false)
    {
        return false;
    }
    video.set_current_time(0.6);
    if !JsFuture::from(race_timeout(once(&video, "seeked")))
        .await
        .map(|settled| settled == JsValue::TRUE)
        .unwrap_or(false)
    {
        return false;
    }
    draw_pixels(document, &video).is_ok()
}

fn parse_key(key_color: &str) -> [u8; 3] {
    let bytes = key_color.as_bytes();
    if bytes.len() == 7 && bytes[0] == b'#' {
        let hex = |index: usize| {
            u8::from_str_radix(key_color.get(index..index + 2).unwrap_or(""), 16).unwrap_or(0)
        };
        [hex(1), hex(3), hex(5)]
    } else {
        [255, 0, 255]
    }
}

/// Shows this roll's dice over the fight: one clip, or two different clips for
/// advantage/disadvantage with the counted one marked, large and centred over the battlefield.
/// Only one roll's dice are ever on screen: the director holds the next blow back until this
/// one's hold has run, and anything still standing is taken down before new dice start.
/// Returns the kept clip's rest time in milliseconds for the land cue and card reveal, or 0
/// when only the resting frame shows.
pub(super) fn show_roll(
    client: &Rc<RefCell<Client>>,
    play: DicePlay,
    short: bool,
) -> Result<u64, JsValue> {
    preload();
    let (document, host, panel) = {
        let state = client.borrow();
        let Some((host, kind)) = dice_host(&state) else {
            return Ok(0);
        };
        (
            state.document.clone(),
            host,
            kind == crate::dice_roll::DiceHost::Panel,
        )
    };
    remove_standing_dice(client)?;
    clear(client)?;
    let Some((first, second, path)) = LIBRARY.with(|library| {
        let mut library = library.borrow_mut();
        if library.clips.is_empty() {
            return None;
        }
        let first = crate::dice_roll::choose_clip(&play, library.clips.len(), library.last);
        let second = play
            .other
            .map(|_| crate::dice_roll::pair_clip(first, library.clips.len()));
        library.last = Some(first);
        Some((first, second, library.path))
    }) else {
        mark_rest_band(
            client,
            super::live_motion::development_duration(
                "dice-tumble",
                crate::dice_roll::ATTACK_D20_TUMBLE_MS,
            ),
        );
        fallback_stills(&document, &host, panel, &play, short)?;
        return Ok(super::live_motion::development_duration(
            "dice-tumble",
            crate::dice_roll::ATTACK_D20_TUMBLE_MS,
        ));
    };
    let overlay = document.create_element("div")?;
    overlay.set_class_name("tv-dice");
    if panel {
        // In flow above the panel's entries: the entries resolve below the landing dice.
        host.insert_before(&overlay, host.first_element_child().as_deref())?;
    } else {
        host.append_child(&overlay)?;
    }
    overlay.set_attribute(
        "data-mode",
        match play.mode {
            RollMode::Plain => "plain",
            RollMode::Advantage => "advantage",
            RollMode::Disadvantage => "disadvantage",
        },
    )?;
    let rest_ms = LIBRARY.with(|library| library.borrow().clips[first].clip.rest_ms);
    let timing = crate::dice_roll::d20_timing(rest_ms, short);
    overlay.set_attribute(
        "style",
        &format!(
            "--dice-rest:{}ms;--dice-hold:{}ms",
            timing.land_ms,
            timing.clear_ms - timing.land_ms - crate::dice_roll::DICE_FADE_MS
        ),
    )?;
    mark_rest_band(client, timing.land_ms);
    let still = should_show_still(path);
    die(
        &document,
        &overlay,
        first,
        play.kept,
        true,
        play.natural,
        still,
    )?;
    if let (Some(other), Some(second)) = (play.other, second) {
        die(&document, &overlay, second, other, false, None, still)?;
    }
    schedule_clear(&overlay, timing.clear_ms)?;
    Ok(timing.land_ms)
}

/// A roll shown while the display catches up still tumbles (FPX-01: nearly every roll arrives
/// that way). Only a missing playable clip leaves the resting frame, which is animated; the
/// machine's reduced-motion preference does not (see the module notes).
fn should_show_still(path: DicePath) -> bool {
    path == DicePath::CardOnly
}

/// Takes down every die and damage die on the display now, ready or not: a new roll never
/// shares the screen with the last one's.
fn remove_standing_dice(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let root = client.borrow().root.clone();
    let standing = root.query_selector_all(".tv-dice, .tv-damage-dice")?;
    for index in 0..standing.length() {
        if let Some(shown) = standing.item(index) {
            let shown: Element = shown.dyn_into().map_err(JsValue::from)?;
            shown.remove();
        }
    }
    Ok(())
}

/// Shows the server's damage faces after a hit, in the same roll card as its d20.
pub(super) fn show_damage(client: &Rc<RefCell<Client>>, faces: &[u32]) -> Result<(), JsValue> {
    let Some(play) = crate::dice_roll::damage_play_of(faces) else {
        return Ok(());
    };
    let (document, host, kind) = {
        let state = client.borrow();
        let Some((host, kind)) = dice_host(&state) else {
            return Ok(());
        };
        (state.document.clone(), host, kind)
    };
    let timing = crate::dice_roll::damage_timing();
    let overlay = document.create_element("div")?;
    overlay.set_class_name("tv-damage-dice");
    overlay.set_attribute(
        "data-host",
        match kind {
            crate::dice_roll::DiceHost::Battlefield => "battlefield",
            crate::dice_roll::DiceHost::Panel => "panel",
            crate::dice_roll::DiceHost::Stage => "stage",
        },
    )?;
    overlay.set_attribute(
        "style",
        &format!(
            "--damage-roll-ms:{}ms",
            super::live_motion::development_duration("damage-dice-tumble", timing.land_ms)
        ),
    )?;
    match kind {
        crate::dice_roll::DiceHost::Battlefield => {
            if let Some(words) = host.query_selector(".bf-roll-words")? {
                host.insert_before(&overlay, Some(&words))?;
            } else {
                host.append_child(&overlay)?;
            }
        }
        crate::dice_roll::DiceHost::Panel => {
            host.insert_before(&overlay, host.first_element_child().as_deref())?;
        }
        crate::dice_roll::DiceHost::Stage => {
            host.append_child(&overlay)?;
        }
    }
    for face in &play.faces {
        let die = document.create_element("div")?;
        die.set_class_name("tv-damage-die");
        die.set_text_content(Some(&face.to_string()));
        overlay.append_child(&die)?;
    }
    Ok(())
}

fn schedule_clear(overlay: &Element, clear_ms: u64) -> Result<(), JsValue> {
    let Some(window) = web_sys::window() else {
        overlay.set_attribute("data-clear-ready", "true")?;
        return Ok(());
    };
    let target = overlay.clone();
    let callback = Closure::wrap(Box::new(move || {
        let _ready = target.set_attribute("data-clear-ready", "true");
        if target.has_attribute("data-clear-pending")
            && !target.has_attribute("data-presentation-owned")
            && let Some(parent) = target.parent_node()
            && let Ok(parent) = parent.dyn_into::<Element>()
        {
            let _removed = parent.remove_child(&target);
        }
    }) as Box<dyn FnMut()>);
    let id = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        callback.as_ref().unchecked_ref(),
        clear_ms.min(i32::MAX as u64) as i32,
    )?;
    LIBRARY.with(|library| {
        let mut library = library.borrow_mut();
        library.timeouts.push(id);
        library.held.push(callback);
    });
    Ok(())
}

fn fallback_stills(
    document: &Document,
    host: &Element,
    panel: bool,
    play: &DicePlay,
    short: bool,
) -> Result<(), JsValue> {
    let overlay = document.create_element("div")?;
    overlay.set_class_name("tv-dice");
    overlay.set_attribute(
        "data-mode",
        match play.mode {
            RollMode::Plain => "plain",
            RollMode::Advantage => "advantage",
            RollMode::Disadvantage => "disadvantage",
        },
    )?;
    overlay.set_attribute(
        "style",
        &format!(
            "--dice-rest:{}ms;--dice-hold:{}ms",
            super::live_motion::development_duration(
                "dice-tumble",
                crate::dice_roll::ATTACK_D20_TUMBLE_MS
            ),
            crate::dice_roll::fight_beat_ms(short)
                .saturating_sub(super::live_motion::development_duration(
                    "dice-tumble",
                    crate::dice_roll::ATTACK_D20_TUMBLE_MS
                ))
                .saturating_sub(crate::dice_roll::DICE_FADE_MS)
        ),
    )?;
    if panel {
        host.insert_before(&overlay, host.first_element_child().as_deref())?;
    } else {
        host.append_child(&overlay)?;
    }
    let first = crate::dice_roll::choose_clip(play, crate::dice_roll::FALLBACK_CLIP_COUNT, None);
    fallback_still(document, &overlay, play.kept, play.natural, true, first)?;
    if let Some(other) = play.other {
        let second = crate::dice_roll::pair_clip(first, crate::dice_roll::FALLBACK_CLIP_COUNT);
        fallback_still(document, &overlay, other, None, false, second)?;
    }
    schedule_clear(&overlay, crate::dice_roll::d20_timing(0, short).clear_ms)?;
    Ok(())
}

fn fallback_still(
    document: &Document,
    overlay: &Element,
    face: u8,
    natural: Option<Natural>,
    kept: bool,
    clip: usize,
) -> Result<(), JsValue> {
    let die = node(document, overlay, "div", "tv-die", "")?;
    die.set_attribute("data-kept", kept_attribute(kept))?;
    die.set_attribute("data-still", "true")?;
    die.set_attribute("style", "outline:none")?;
    let image = node(document, &die, "img", "tv-die-rest", "")?;
    image.set_attribute("src", &crate::dice_roll::fallback_rest_url(clip))?;
    image.set_attribute("alt", "")?;
    image.set_attribute("style", "display:block")?;
    let figure = node(document, &die, "div", "tv-die-num", &face.to_string())?;
    if let Some(natural) = natural {
        figure.set_attribute(
            "data-nat",
            match natural {
                Natural::Twenty => "20",
                Natural::One => "1",
            },
        )?;
    }
    figure.set_attribute(
        "style",
        "left:50%;top:49%;width:30%;height:30%;font-size:calc(var(--u)*4);opacity:0!important;transform:translate(-50%,-50%)",
    )?;
    let settled = figure.clone();
    let die = die.clone();
    let _marked = die.set_attribute("data-fallback-tumble", "true");
    spawn_local(async move {
        TimeoutFuture::new(super::live_motion::development_duration(
            "dice-tumble",
            crate::dice_roll::ATTACK_D20_TUMBLE_MS,
        ) as u32)
        .await;
        if !settled.is_connected() {
            return;
        }
        let _ = die.remove_attribute("data-fallback-tumble");
        let _ = settled.set_attribute("data-settled", "true");
        let _ = settled.set_attribute("style", "left:50%;top:49%;width:30%;height:30%;font-size:calc(var(--u)*4);opacity:1!important;transform:translate(-50%,-50%)");
    });
    Ok(())
}

/// Takes dice down after their result read window: both hosts sit under the page root, so
/// one pass handles every shown die wherever it was hosted.
pub(super) fn clear(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
    let root = client.borrow().root.clone();
    let previous = root.query_selector_all(".tv-dice, .tv-damage-dice")?;
    let mut waiting_for_roll = false;
    for index in 0..previous.length() {
        if let Some(shown) = previous.item(index) {
            let shown: Element = shown.dyn_into().map_err(JsValue::from)?;
            if shown.has_attribute("data-presentation-owned") {
                waiting_for_roll = true;
                continue;
            }
            if shown.has_attribute("data-clear-ready") {
                if let Some(parent) = shown.parent_node()
                    && let Ok(parent) = parent.dyn_into::<Element>()
                {
                    parent.remove_child(&shown)?;
                }
            } else {
                shown.set_attribute("data-clear-pending", "true")?;
                waiting_for_roll = true;
            }
        }
    }
    if waiting_for_roll {
        return Ok(());
    }
    if let Some(window) = web_sys::window() {
        LIBRARY.with(|library| {
            for id in library.borrow_mut().timeouts.drain(..) {
                window.clear_timeout_with_handle(id);
            }
        });
    }
    LIBRARY.with(|library| {
        let mut library = library.borrow_mut();
        library.held.clear();
        library.listeners.clear();
        library.painters.clear();
    });
    Ok(())
}

/// The rest-image URL for a roll without playback history: the phone's small "Latest
/// roll" card. `None` until the manifest has loaded; the badge then reads as before.
pub(super) fn rest_url_for(attack_die: u32, faces: &[u32], roll_mode: i32) -> Option<String> {
    preload();
    let play = crate::dice_roll::play_of(attack_die, faces, roll_mode)?;
    LIBRARY.with(|library| {
        let library = library.borrow();
        if library.clips.is_empty() {
            return None;
        }
        let index = crate::dice_roll::choose_clip(&play, library.clips.len(), None);
        Some(crate::dice_roll::clip_url(
            &library.clips[index].clip.rest_image,
        ))
    })
}

/// The element the dice are hosted in: beside the battlefield's attack card while it is
/// mounted, else the flat roll panel in flow, else the stage root.
fn dice_host(state: &Client) -> Option<(Element, crate::dice_roll::DiceHost)> {
    use crate::dice_roll::{DiceHost, dice_host as host_for};
    let field_mounted = state.root.query_selector(".battlefield").ok()?.is_some();
    let panel = state
        .document
        .query_selector(".live-screen[data-role=display] .combat-roll-panel")
        .ok()
        .flatten();
    let kind = host_for(field_mounted, panel.is_some());
    let host = match kind {
        // The battlefield's own container: above its canvas, below its HUD, centred.
        DiceHost::Battlefield => state.root.query_selector(".battlefield").ok()??,
        DiceHost::Panel => panel?,
        DiceHost::Stage => state.live.tv.as_ref().map(|stage| stage.root().clone())?,
    };
    Some((host, kind))
}

/// Rounds the kept rest time to a 100 ms band for the roll-card reveal delay, so the
/// stylesheet needs no inline arithmetic; 0 clears it back to the instant default.
fn mark_rest_band(client: &Rc<RefCell<Client>>, rest_ms: u64) {
    let host = client
        .borrow()
        .root
        .query_selector(".live-screen")
        .ok()
        .flatten();
    if let Some(host) = host {
        let band = if rest_ms == 0 {
            String::new()
        } else {
            format!("{}", (rest_ms / 100 * 100).clamp(1_100, 1_600))
        };
        if band.is_empty() {
            let _cleared = host.remove_attribute("data-rest");
        } else {
            let _marked = host.set_attribute("data-rest", &band);
        }
    }
}

fn attack_card_visibility(action: &crate::dice_roll::AttackDisplayAction) -> Option<&'static str> {
    use crate::dice_roll::AttackDisplayAction;

    match action {
        AttackDisplayAction::OpenCard => Some("open"),
        AttackDisplayAction::Close | AttackDisplayAction::Hide => Some("closed"),
        _ => None,
    }
}

fn set_attack_card_visibility(root: &Element, state: &str) {
    if let Ok(Some(screen)) = root.query_selector(".live-screen[data-role=display]") {
        if state == "open" {
            let _ = screen.set_attribute("data-attack-card", "open");
        } else {
            let _ = screen.remove_attribute("data-attack-card");
        }
    }
}

fn die(
    document: &Document,
    parent: &Element,
    clip: usize,
    face: u8,
    kept: bool,
    natural: Option<Natural>,
    still: bool,
) -> Result<(), JsValue> {
    let settled_now = false;
    let die = node(document, parent, "div", "tv-die", "")?;
    die.set_attribute("data-kept", kept_attribute(kept))?;
    die.set_attribute("style", "outline:none")?;
    if let Some(natural) = natural {
        die.set_attribute(
            "data-nat",
            match natural {
                Natural::Twenty => "20",
                Natural::One => "1",
            },
        )?;
    }
    let path = LIBRARY.with(|library| library.borrow().path);
    // The resting frame rides hidden underneath; a stalled clip hands over to it.
    let video = if still || path == DicePath::CardOnly {
        rest_image(document, &die, clip)?;
        die.set_attribute("data-still", "true")?;
        None
    } else if path == DicePath::Key {
        let video = keyed_clip(document, &die, clip)?;
        rest_image(document, &die, clip)?;
        Some(video)
    } else {
        let video = web_clip(document, &die, clip)?;
        rest_image(document, &die, clip)?;
        Some(video)
    };
    stall_watch(&die, video.as_ref())?;
    number(document, &die, clip, face, natural, settled_now, video)?;
    Ok(())
}

fn kept_attribute(kept: bool) -> &'static str {
    if kept { "true" } else { "false" }
}

/// A clip that errors hands over to the resting frame at once instead of standing black.
fn stall_watch(die: &Element, video: Option<&web_sys::HtmlVideoElement>) -> Result<(), JsValue> {
    let Some(video) = video else {
        return Ok(());
    };
    let stalled = die.clone();
    let failed = Closure::wrap(Box::new(move |_: web_sys::Event| {
        let _marked = stalled.set_attribute("data-stalled", "true");
    }) as Box<dyn FnMut(web_sys::Event)>);
    video.add_event_listener_with_callback("error", failed.as_ref().unchecked_ref())?;
    LIBRARY.with(|library| library.borrow_mut().listeners.push(failed));
    Ok(())
}

/// The number on the upward face, positioned, scaled and rotated from the manifest's face
/// geometry; only the server's face ever reads here. Appears at the rest time, or at once
/// for a still die.
fn number(
    document: &Document,
    die: &Element,
    clip: usize,
    face: u8,
    natural: Option<Natural>,
    settled_now: bool,
    video: Option<web_sys::HtmlVideoElement>,
) -> Result<(), JsValue> {
    let (center, corners, rotation, _rest_ms) = LIBRARY.with(|library| {
        let library = library.borrow();
        let clip = &library.clips[clip].clip;
        (
            clip.face_center,
            clip.face_corners,
            clip.face_rotation_deg,
            clip.rest_ms,
        )
    });
    let figure = node(document, die, "div", "tv-die-num", &face.to_string())?;
    if let Some(natural) = natural {
        figure.set_attribute(
            "data-nat",
            match natural {
                Natural::Twenty => "20",
                Natural::One => "1",
            },
        )?;
    }
    let min_x = corners
        .iter()
        .map(|corner| corner[0])
        .fold(f64::INFINITY, f64::min);
    let max_x = corners
        .iter()
        .map(|corner| corner[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = corners
        .iter()
        .map(|corner| corner[1])
        .fold(f64::INFINITY, f64::min);
    let max_y = corners
        .iter()
        .map(|corner| corner[1])
        .fold(f64::NEG_INFINITY, f64::max);
    let mut edge = 0.0f64;
    for pair in [(0, 1), (1, 2), (2, 0)] {
        let (a, b) = (corners[pair.0], corners[pair.1]);
        edge = edge.max(((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt());
    }
    // The laid-out die box turns the face fraction into pixels; unreadable without one.
    let pixels = die.client_width().max(1) as f64 * edge * 0.5;
    let figure_style = format!(
        "left:{:.2}%;top:{:.2}%;width:{:.2}%;height:{:.2}%;font-size:{:.0}px;opacity:{}!important;transform:translate(-50%,-50%) rotate({:.1}deg)",
        center[0] * 100.0,
        center[1] * 100.0,
        (max_x - min_x) * 100.0,
        (max_y - min_y) * 100.0,
        pixels,
        if settled_now { 1 } else { 0 },
        rotation,
    );
    figure.set_attribute("style", &figure_style)?;
    if settled_now {
        figure.set_attribute("data-settled", "true")?;
        return Ok(());
    }
    let settled = figure.clone();
    let landed = die.clone();
    let settled_style = figure_style.replace("opacity:0", "opacity:1");
    // At the rest time the number appears; a clip with no data yet hands over to the
    // resting frame instead of standing black behind the number. The timer belongs to this
    // die: a later cue taking other dice down never cancels it, and a die that has gone by
    // then is left alone.
    let rest = super::live_motion::development_duration(
        "dice-tumble",
        crate::dice_roll::ATTACK_D20_TUMBLE_MS,
    ) as u32;
    spawn_local(async move {
        TimeoutFuture::new(rest).await;
        if !settled.is_connected() {
            return;
        }
        let _shown = settled.set_attribute("data-settled", "true");
        let _styled = settled.set_attribute("style", &settled_style);
        if video.as_ref().is_some_and(|clip| clip.ready_state() < 3) {
            let _marked = landed.set_attribute("data-stalled", "true");
        }
    });
    Ok(())
}

fn web_clip(
    document: &Document,
    die: &Element,
    clip: usize,
) -> Result<web_sys::HtmlVideoElement, JsValue> {
    let url = LIBRARY.with(|library| library.borrow().clips[clip].webm_url.clone());
    let video: web_sys::HtmlVideoElement = document.create_element("video")?.dyn_into()?;
    video.set_class_name("tv-die-clip");
    video.set_attribute("style", "display:block!important")?;
    video.set_muted(true);
    video.set_attribute("muted", "")?;
    video.set_attribute("playsinline", "")?;
    video.set_attribute("preload", "auto")?;
    video.set_playback_rate(LIBRARY.with(|library| {
        library.borrow().clips[clip].clip.rest_ms as f64
            / super::live_motion::development_duration(
                "dice-tumble",
                crate::dice_roll::ATTACK_D20_TUMBLE_MS,
            ) as f64
    }));
    video.set_src(&url);
    die.append_child(&video)?;
    // A clip that refuses to start leaves its resting frame (already underneath) and the
    // number, which still appears on schedule: the beat never waits on the video.
    let _started = video.play();
    Ok(video)
}

fn rest_image(document: &Document, die: &Element, clip: usize) -> Result<(), JsValue> {
    let url = LIBRARY.with(|library| library.borrow().clips[clip].rest_url.clone());
    let image = document.create_element("img")?;
    image.set_class_name("tv-die-rest");
    image.set_attribute("src", &url)?;
    image.set_attribute("alt", "")?;
    image.set_attribute("draggable", "false")?;
    die.append_child(&image)?;
    Ok(())
}

/// The key-colour path: the MP4 plays hidden while each frame is drawn through a canvas
/// with the key removed. The resting frame sits underneath, so a stalled loop still
/// leaves a settled die with its number.
fn keyed_clip(
    document: &Document,
    die: &Element,
    clip: usize,
) -> Result<web_sys::HtmlVideoElement, JsValue> {
    let url = LIBRARY.with(|library| library.borrow().clips[clip].key_url.clone());
    let canvas = document.create_element("canvas")?;
    canvas.set_class_name("tv-die-canvas");
    canvas.set_attribute("style", "display:block!important")?;
    canvas.set_attribute("width", &CLIP_SIZE.to_string())?;
    canvas.set_attribute("height", &CLIP_SIZE.to_string())?;
    die.append_child(&canvas)?;
    let video: web_sys::HtmlVideoElement = document.create_element("video")?.dyn_into()?;
    video.set_class_name("tv-die-src");
    video.set_muted(true);
    video.set_attribute("muted", "")?;
    video.set_attribute("playsinline", "")?;
    video.set_attribute("preload", "auto")?;
    video.set_playback_rate(LIBRARY.with(|library| {
        library.borrow().clips[clip].clip.rest_ms as f64
            / super::live_motion::development_duration(
                "dice-tumble",
                crate::dice_roll::ATTACK_D20_TUMBLE_MS,
            ) as f64
    }));
    video.set_src(&url);
    die.append_child(&video)?;
    let key = LIBRARY.with(|library| library.borrow().key);
    let keyed = video.clone();
    let frame = Rc::new(RefCell::new(None::<Closure<dyn FnMut(f64)>>));
    let step = frame.clone();
    let paint = Closure::wrap(Box::new(move |_: f64| {
        let alive = canvas.is_connected() && !keyed.ended();
        if alive {
            key_frame(&canvas, &keyed, key);
        }
        if alive
            && let Some(window) = web_sys::window()
            && let Some(next) = step.borrow().as_ref()
        {
            let _queued = window.request_animation_frame(next.as_ref().unchecked_ref());
        }
    }) as Box<dyn FnMut(f64)>);
    *frame.borrow_mut() = Some(paint);
    let queued = web_sys::window()
        .and_then(|window| {
            frame.borrow().as_ref().and_then(|first| {
                window
                    .request_animation_frame(first.as_ref().unchecked_ref())
                    .ok()
            })
        })
        .is_some();
    if queued && let Some(painter) = frame.borrow_mut().take() {
        LIBRARY.with(|library| library.borrow_mut().painters.push(painter));
    }
    let _started = video.play();
    Ok(video)
}

/// One keyed frame: draw the video, drop pixels near the key colour (feathered), cut the
/// magenta fringe off what stays, put it back. Silence on failure: the resting frame
/// underneath covers a missed frame.
fn key_frame(canvas: &Element, video: &web_sys::HtmlVideoElement, key: [u8; 3]) {
    let Ok(context) = canvas_context(canvas) else {
        return;
    };
    if call(
        &context,
        "drawImage",
        &[
            JsValue::from(video.clone()),
            JsValue::from(0.0),
            JsValue::from(0.0),
        ],
    )
    .is_err()
    {
        return;
    }
    let size = f64::from(CLIP_SIZE);
    let Ok(image) = call(
        &context,
        "getImageData",
        &[
            JsValue::from(0.0),
            JsValue::from(0.0),
            size.into(),
            size.into(),
        ],
    ) else {
        return;
    };
    let Some(data) = Reflect::get(&image, &JsValue::from_str("data"))
        .ok()
        .and_then(|datum| datum.dyn_into::<Uint8ClampedArray>().ok())
    else {
        return;
    };
    let mut pixels = data.to_vec();
    let (kr, kg, kb) = (i32::from(key[0]), i32::from(key[1]), i32::from(key[2]));
    // The buffer is a whole number of RGBA pixels; the empty tail never runs.
    let (body, _) = pixels.as_chunks_mut::<4>();
    for pixel in body {
        let distance = (i32::from(pixel[0]) - kr).pow(2)
            + (i32::from(pixel[1]) - kg).pow(2)
            + (i32::from(pixel[2]) - kb).pow(2);
        if distance < KEY_FULL_OUT {
            pixel[3] = 0;
        } else if distance < KEY_FEATHER_OUT {
            pixel[3] = ((distance - KEY_FULL_OUT) * 255 / (KEY_FEATHER_OUT - KEY_FULL_OUT)) as u8;
        } else {
            // Spill removal for the kept edge: the smaller of red/blue above green is
            // fringe from the key, not the die (gold and navy both carry more green).
            let fringe = pixel[0].min(pixel[2]).saturating_sub(pixel[1]) / 2;
            pixel[0] -= fringe;
            pixel[2] -= fringe;
        }
    }
    data.copy_from(&pixels);
    let _shown = call(
        &context,
        "putImageData",
        &[image, JsValue::from(0.0), JsValue::from(0.0)],
    );
}

/// Draws a video frame to an offscreen canvas and returns its RGBA bytes, for the
/// start-up probe.
fn draw_pixels(document: &Document, video: &web_sys::HtmlVideoElement) -> Result<Vec<u8>, JsValue> {
    let canvas = document.create_element("canvas")?;
    canvas.set_attribute("width", &CLIP_SIZE.to_string())?;
    canvas.set_attribute("height", &CLIP_SIZE.to_string())?;
    let context = canvas_context(&canvas)?;
    call(
        &context,
        "drawImage",
        &[
            JsValue::from(video.clone()),
            JsValue::from(0.0),
            JsValue::from(0.0),
        ],
    )?;
    let size = f64::from(CLIP_SIZE);
    let image = call(
        &context,
        "getImageData",
        &[
            JsValue::from(0.0),
            JsValue::from(0.0),
            size.into(),
            size.into(),
        ],
    )?;
    let data: Uint8ClampedArray = Reflect::get(&image, &JsValue::from_str("data"))?.dyn_into()?;
    Ok(data.to_vec())
}

/// The 2D context through untyped calls (see the module docs for why).
fn canvas_context(canvas: &Element) -> Result<JsValue, JsValue> {
    let get: Function = Reflect::get(canvas, &JsValue::from_str("getContext"))?.dyn_into()?;
    get.call1(canvas, &JsValue::from_str("2d"))
}

fn call(context: &JsValue, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let method: Function = Reflect::get(context, &JsValue::from_str(name))?.dyn_into()?;
    method.apply(context, &args.iter().collect::<Array>())
}

/// The rest time a roll shown now should budget before the clips' own times are known: the
/// slowest loaded clip's, else the default.
pub(super) fn expected_rest_ms() -> u64 {
    super::live_motion::development_duration("dice-tumble", crate::dice_roll::ATTACK_D20_TUMBLE_MS)
}

/// Component ids of a fight blow's dice start here, so they never collide with a dice line's
/// (those are the line machine's ids); the low bits are the blow's server sequence.
const FIGHT_COMPONENT_BASE: u64 = 1 << 40;

/// Owns the actual attack overlay/card lifecycle. Each await checks the original connected
/// overlay so a replacement roll or a scene teardown cannot receive stale mutations.
pub(super) fn log_fight_roll(
    client: &Rc<RefCell<Client>>,
    sequence: u64,
    lag_ms: u64,
    _rest_ms: u64,
    _short: bool,
) {
    use crate::dice_roll::AttackDisplayAction;
    // Cue entry precedes Render: the accepted view already contains this result, while the
    // battlefield's cache can still contain only the preceding view.
    let outcome = {
        let state = client.borrow();
        state
            .views
            .as_ref()
            .and_then(|views| views.current())
            .and_then(|view| {
                let rpc::view_message::Audience::Display(display) = view.audience.as_ref()? else {
                    return None;
                };
                crate::dice_roll::live_attack(display.journey.as_ref()?.combat.as_ref()?, sequence)
                    .cloned()
            })
    }
    .or_else(|| super::live_battlefield::attack_outcome(sequence));
    let Some(outcome) = outcome else {
        return;
    };
    if outcome
        .presentation
        .as_ref()
        .is_some_and(crate::dice_roll::technique_owns_roll)
    {
        if let Ok(Some(dice)) = client.borrow().root.query_selector(".tv-dice") {
            dice.remove();
        }
        return;
    }
    let Some(mut display) = crate::dice_roll::AttackDisplay::new(&outcome) else {
        return;
    };
    let (root, scene) = {
        let state = client.borrow();
        (
            state.root.clone(),
            state
                .root
                .get_attribute("data-battlefield-scene")
                .unwrap_or_default(),
        )
    };
    // start_roll normally mounted this die. Keep the live entry complete when a caller has
    // only the committed outcome (and never add a second die to an existing seat).
    if root.query_selector(".tv-dice").ok().flatten().is_none()
        && show_roll(client, display.play, false).is_err()
    {
        return;
    }
    let Some(dice) = root.query_selector(".tv-dice").ok().flatten() else {
        return;
    };
    let reason = outcome.reasons.join("; ");
    let hit = outcome.hit || outcome.grazed;
    super::live_battlefield::begin_attack_presentation(outcome);
    let _ = dice.set_attribute("data-sequence", &sequence.to_string());
    let _ = dice.set_attribute("data-presentation-owned", "true");
    let _ = dice.set_attribute("style", "animation:none");
    if !reason.is_empty() {
        let _ = dice.set_attribute("data-reason", &reason);
    }
    let settled_ms =
        super::live_motion::development_duration("dice-tumble", display.timing.settled_ms);
    let card_ms = settled_ms
        + super::live_motion::development_duration(
            "dice-read",
            crate::dice_roll::ATTACK_FACE_DWELL_MS,
        );
    let damage_ms = super::live_motion::development_duration(
        "damage-dice-tumble",
        df_render::battle::DAMAGE_DICE_TUMBLE.as_millis() as u64,
    );
    let hold = super::live_motion::development_duration(
        if hit {
            "roll-card-hit-hold"
        } else {
            "roll-card-miss-hold"
        },
        if hit {
            df_render::battle::ROLL_CARD_HIT_HOLD
        } else {
            df_render::battle::ROLL_CARD_MISS_HOLD
        }
        .as_millis() as u64,
    ) as u32;
    let has_damage = !display.damage_faces.is_empty();
    let closing_ms = card_ms + if has_damage { damage_ms } else { 0 } + u64::from(hold);
    display.timing = crate::dice_roll::AttackTiming {
        settled_ms,
        card_ms,
        damage_settled_ms: has_damage.then_some(card_ms + damage_ms),
        closing_ms,
        hidden_ms: closing_ms + crate::dice_roll::DICE_FADE_MS,
    };
    super::presentation_timing::note_lag(lag_ms);
    let id = FIGHT_COMPONENT_BASE | sequence;
    let client = Rc::clone(client);
    spawn_local(async move {
        let mut previous_ms = 0;
        let mut card = None;
        let mut damage = None;
        let completed = async {
            for (at_ms, action) in display.steps() {
                TimeoutFuture::new(at_ms.saturating_sub(previous_ms) as u32).await;
                previous_ms = at_ms;
                if !dice.is_connected()
                    || !super::live_battlefield::ordinary_presentation_active(sequence)
                {
                    return false;
                }
                if let Some(state) = attack_card_visibility(&action) {
                    set_attack_card_visibility(&root, state);
                }
                match action {
                    AttackDisplayAction::OpenDie
                    | AttackDisplayAction::SettleDie
                    | AttackDisplayAction::ReadCard => {}
                    AttackDisplayAction::OpenCard => {
                        if !super::live_battlefield::show_attack_card(
                            sequence,
                            display.attack_card.clone(),
                        ) {
                            return false;
                        }
                        // A card request is not a visible opening: wait for the renderer's DOM.
                        for _ in 0..10 {
                            TimeoutFuture::new(16).await;
                            if !dice.is_connected() {
                                return false;
                            }
                            card = root.query_selector(".bf-roll[data-open]").ok().flatten();
                            if card.is_some() {
                                break;
                            }
                        }
                        if card.is_none() {
                            return false;
                        }
                        if !has_damage {
                            super::live_battlefield::show_attack_card(
                                sequence,
                                display.damage_card.clone(),
                            );
                            super::live_battlefield::damage_readable(sequence);
                        }
                    }
                    AttackDisplayAction::OpenDamage => {
                        if show_damage(&client, &display.damage_faces).is_err() {
                            return false;
                        }
                        damage = root.query_selector(".tv-damage-dice").ok().flatten();
                    }
                    AttackDisplayAction::SettleDamage => {
                        if let Some(damage) = &damage {
                            let _ = damage.set_attribute("data-settled", "true");
                        }
                        if !super::live_battlefield::show_attack_card(
                            sequence,
                            display.damage_card.clone(),
                        ) {
                            return false;
                        }
                        super::live_battlefield::damage_readable(sequence);
                    }
                    AttackDisplayAction::Close => {
                        // Retain ownership through the fade; no generic clear may remove the die
                        // before the corresponding Hidden records are emitted.
                        super::live_battlefield::close_attack_card(sequence);
                        let _ = dice.set_attribute("style", "opacity:0;transition:opacity 320ms");
                        if let Some(damage) = &damage {
                            let _ = damage.set_attribute("data-closing", "true");
                        }
                    }
                    AttackDisplayAction::Hide => {
                        dice.remove();
                        if let Some(damage) = &damage {
                            damage.remove();
                        }
                        super::live_battlefield::finish_attack_presentation(sequence);
                    }
                }
                for (component, phase, reason) in action.records(has_damage) {
                    let (element, component_id) = match component {
                        PresentationComponent::Die => (Some(&dice), id),
                        PresentationComponent::Dice => (damage.as_ref(), id ^ 0xD000_0000),
                        _ => (card.as_ref(), id),
                    };
                    record_fight_component(
                        element,
                        component,
                        component_id,
                        &scene,
                        phase,
                        reason,
                        hold,
                    );
                }
            }
            true
        }
        .await;
        if !completed {
            // A missing HUD, replaced sequence or disconnected scene cancels this owner;
            // it must not leave a permanently owned overlay or hold the next live attack.
            dice.remove();
            if let Some(damage) = &damage {
                damage.remove();
            }
            super::live_battlefield::finish_attack_presentation(sequence);
            for (component, _, _) in AttackDisplayAction::Hide.records(has_damage) {
                let (element, component_id) = match component {
                    PresentationComponent::Die => (Some(&dice), id),
                    PresentationComponent::Dice => (damage.as_ref(), id ^ 0xD000_0000),
                    _ => (None, id),
                };
                record_fight_component(
                    element,
                    component,
                    component_id,
                    &scene,
                    ComponentPhase::Hidden,
                    "attack-cancelled",
                    hold,
                );
            }
        }
    });
}

fn record_fight_component(
    element: Option<&Element>,
    component: PresentationComponent,
    id: u64,
    scene: &str,
    phase: ComponentPhase,
    reason: &'static str,
    read_ms: u32,
) {
    super::presentation_timing::component(element, component, id, scene, phase, reason, read_ms);
}

/// Applies a dice line's state: the die and result card open with the line, are marked closing
/// when it fades, and are taken down when it is over. The line machine owns the hold: the
/// card stays for exactly the line's `read_ms`, so the next line never starts before it ends.
pub(super) fn line_state(
    client: &Rc<RefCell<Client>>,
    id: u64,
    state: LineState,
    card: &RollCard,
    interrupted: bool,
) -> Result<(), JsValue> {
    match state {
        LineState::Opening => open_card(client, id, card),
        LineState::Closing => {
            set_card_phase(client, id, "closing", ComponentPhase::Closing, "read");
            Ok(())
        }
        LineState::Done | LineState::Cancelled => {
            finish_card(client, id, interrupted);
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The element a roll's dice and card are hosted in: the 3D battlefield's container while it
/// is mounted (above its canvas, below its HUD), else the stage root, whose stacking puts the
/// card above the plate and below the subtitle (see `live_tv.css`).
fn card_host(state: &Client) -> Option<Element> {
    state
        .root
        .query_selector(".battlefield")
        .ok()
        .flatten()
        .or_else(|| state.live.tv.as_ref().map(|stage| stage.root().clone()))
}

fn card_element(client: &Rc<RefCell<Client>>, id: u64) -> Option<Element> {
    client
        .borrow()
        .root
        .query_selector(&format!(".tv-roll[data-line-id=\"{id}\"]"))
        .ok()
        .flatten()
}

/// One lifecycle record of a roll's presentation in the DISPLAY log, through the shared
/// component path: opened, readable (the result card showing), closing and hidden carry the
/// opened, closed and held times.
fn record(
    element: Option<&Element>,
    id: u64,
    phase: ComponentPhase,
    reason: &'static str,
    read_ms: u32,
) {
    super::presentation_timing::component(
        element,
        PresentationComponent::Dice,
        id,
        "",
        phase,
        reason,
        read_ms,
    );
}

fn set_card_phase(
    client: &Rc<RefCell<Client>>,
    id: u64,
    phase: &str,
    component: ComponentPhase,
    reason: &'static str,
) {
    if let Some(element) = card_element(client, id) {
        let _marked = element.set_attribute("data-phase", phase);
        record(Some(&element), id, component, reason, 0);
    }
}

fn finish_card(client: &Rc<RefCell<Client>>, id: u64, interrupted: bool) {
    if let Some(element) = card_element(client, id) {
        element.remove();
    }
    record(
        None,
        id,
        ComponentPhase::Hidden,
        if interrupted { "replaced" } else { "read" },
        0,
    );
}

/// Opens a roll's dice and result card: the dice tumble from the clips (two for Advantage or
/// Disadvantage, the counted one marked) and the server's number lands on the face at the
/// clip's rest time, when the result card appears with its label, sum and verdict. Both are
/// the server's numbers, composed by [`RollCard`]; the browser decides nothing.
fn open_card(client: &Rc<RefCell<Client>>, id: u64, card: &RollCard) -> Result<(), JsValue> {
    preload();
    let (document, host) = {
        let state = client.borrow();
        let Some(host) = card_host(&state) else {
            return Ok(());
        };
        (state.document.clone(), host)
    };
    // One roll at a time on this layer: a card still standing is gone before this one opens.
    let standing = host.query_selector_all(".tv-roll")?;
    for index in 0..standing.length() {
        if let Some(shown) = standing.item(index) {
            let shown: Element = shown.dyn_into().map_err(JsValue::from)?;
            shown.remove();
        }
    }
    let play = card.play();
    let picked = LIBRARY.with(|library| {
        let mut library = library.borrow_mut();
        if library.clips.is_empty() {
            return None;
        }
        let first = crate::dice_roll::choose_clip(&play, library.clips.len(), library.last);
        let second = play
            .other
            .map(|_| crate::dice_roll::pair_clip(first, library.clips.len()));
        library.last = Some(first);
        Some((first, second, library.clips[first].clip.rest_ms))
    });
    let rest_ms = picked.map_or(crate::roll_card::DEFAULT_REST_MS, |(_, _, rest)| rest);
    let timing = card.timing(rest_ms, false);
    let overlay = document.create_element("div")?;
    overlay.set_class_name("tv-roll");
    overlay.set_attribute("data-line-id", &id.to_string())?;
    overlay.set_attribute("data-phase", "rolling")?;
    overlay.set_attribute("data-dice", &card.dice.len().to_string())?;
    overlay.set_attribute("data-counted-face", &card.counted_face().to_string())?;
    overlay.set_attribute(
        "data-mode",
        match card.mode {
            RollMode::Plain => "plain",
            RollMode::Advantage => "advantage",
            RollMode::Disadvantage => "disadvantage",
        },
    )?;
    overlay.set_attribute(
        "data-succeeded",
        if card.succeeded { "true" } else { "false" },
    )?;
    overlay.set_attribute("data-rest-ms", &timing.rest_ms.to_string())?;
    overlay.set_attribute("data-hold-ms", &timing.hold_ms.to_string())?;
    overlay.set_attribute(
        "style",
        &format!("--dice-rest:{}ms", timing.rest_ms.min(2_500)),
    )?;
    // Attached first: the number's size is read from the laid-out die.
    host.append_child(&overlay)?;
    let dice = node(&document, &overlay, "div", "tv-roll-dice", "")?;
    match picked {
        Some((first, second, _)) => {
            let path = LIBRARY.with(|library| library.borrow().path);
            let still = should_show_still(path);
            die(
                &document,
                &dice,
                first,
                play.kept,
                true,
                play.natural,
                still,
            )?;
            if let (Some(other), Some(second)) = (play.other, second) {
                die(&document, &dice, second, other, false, None, still)?;
            }
        }
        None => {
            // The clips have not loaded: the resting frames, animated, with the server's faces.
            let count = crate::dice_roll::FALLBACK_CLIP_COUNT;
            let first = crate::dice_roll::choose_clip(&play, count, None);
            fallback_still(&document, &dice, play.kept, play.natural, true, first)?;
            if let Some(other) = play.other {
                let second = crate::dice_roll::pair_clip(first, count);
                fallback_still(&document, &dice, other, None, false, second)?;
            }
        }
    }
    let face = node(&document, &overlay, "div", "tv-roll-card", "")?;
    face.set_attribute("aria-live", "polite")?;
    node(&document, &face, "p", "tv-roll-label", &card.label)?;
    node(&document, &face, "p", "tv-roll-sum", &card.sum_text())?;
    let verdict = node(
        &document,
        &face,
        "p",
        "tv-roll-verdict",
        card.verdict().unwrap_or_default(),
    )?;
    verdict.set_attribute(
        "data-verdict",
        if card.succeeded { "success" } else { "failure" },
    )?;
    record(None, id, ComponentPhase::Requested, "ready", timing.hold_ms);
    record(
        Some(&overlay),
        id,
        ComponentPhase::Opened,
        "ready",
        timing.hold_ms,
    );
    let (result, hold_ms, rest) = (
        overlay.clone(),
        timing.hold_ms,
        u32::try_from(timing.rest_ms).unwrap_or(u32::MAX),
    );
    spawn_local(async move {
        TimeoutFuture::new(rest).await;
        if result.is_connected() && result.get_attribute("data-phase").as_deref() == Some("rolling")
        {
            let _revealed = result.set_attribute("data-phase", "result");
            record(
                Some(&result),
                id,
                ComponentPhase::Readable,
                "ready",
                hold_ms,
            );
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{DicePath, attack_card_visibility, kept_attribute, should_show_still};
    use crate::dice_roll::AttackDisplayAction;

    #[test]
    fn a_playable_clip_plays_whatever_the_motion_preference_says() {
        // The pane that showed only the resting frame reports prefers-reduced-motion: reduce
        // while its alpha probe passes; the clip is the game's one indispensable visual.
        assert!(!should_show_still(DicePath::WebM));
        assert!(!should_show_still(DicePath::Key));
        assert!(should_show_still(DicePath::CardOnly));
    }

    #[test]
    fn only_the_server_selected_die_gets_the_kept_marker() {
        assert_eq!(kept_attribute(true), "true");
        assert_eq!(kept_attribute(false), "false");
    }

    #[test]
    fn the_attack_card_open_phase_sets_and_clears_its_visible_state() {
        assert_eq!(
            attack_card_visibility(&AttackDisplayAction::OpenCard),
            Some("open")
        );
        assert_eq!(
            attack_card_visibility(&AttackDisplayAction::Close),
            Some("closed")
        );
        assert_eq!(
            attack_card_visibility(&AttackDisplayAction::Hide),
            Some("closed")
        );
    }
}
