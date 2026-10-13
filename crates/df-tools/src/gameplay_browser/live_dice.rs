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
//! drawn through a canvas with the key removed, otherwise only the roll card shows. (This
//! machine's Chrome takes the WebM path: headless shell decodes VP9 alpha and the probe
//! below reads a mixed transparent/opaque frame.) The canvas calls are untyped on purpose:
//! the canvas web-sys features would need another worker's `Cargo.toml`, so this one small
//! module calls the half-dozen DOM methods through `js_sys::Reflect` instead.
//!
//! Where the dice live: the 3D battlefield paints over the whole stage (see the stacking
//! order in `live_tv.css`), so while it is mounted the dice are hosted inside its
//! container, above its canvas and below its HUD; on the flat stage they stay on the stage
//! root, below its cards.
use super::*;
use crate::dice_roll::{DiceClip, DiceManifest, DicePlay, Natural, RollMode};
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

async fn fetch_text(url: &str, maximum: u32) -> Option<String> {
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
    if text.is_empty() || text.len() as u32 > maximum {
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

fn parse_key(text: &str) -> [u8; 3] {
    let bytes = text.as_bytes();
    if bytes.len() == 7 && bytes[0] == b'#' {
        let hex = |index: usize| {
            u8::from_str_radix(text.get(index..index + 2).unwrap_or(""), 16).unwrap_or(0)
        };
        [hex(1), hex(3), hex(5)]
    } else {
        [255, 0, 255]
    }
}

/// Shows this roll's dice over the fight, replacing any previous roll's dice: one clip,
/// or two different clips for advantage/disadvantage with the counted one marked. Returns
/// the kept clip's rest time in milliseconds for the land cue and the card reveal, or 0
/// when no clip played and only the roll card shows (the beat is never delayed).
pub(super) fn show_roll(
    client: &Rc<RefCell<Client>>,
    play: DicePlay,
    _short: bool,
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
        mark_rest_band(client, 0);
        fallback_stills(&document, &host, panel, &play)?;
        return Ok(0);
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
    overlay.set_attribute("style", &format!("--dice-rest:{}ms", rest_ms.min(2_500)))?;
    mark_rest_band(client, rest_ms);
    let still = path == DicePath::CardOnly;
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
    Ok(rest_ms)
}

fn fallback_stills(
    document: &Document,
    host: &Element,
    panel: bool,
    play: &DicePlay,
) -> Result<(), JsValue> {
    let overlay = document.create_element("div")?;
    overlay.set_class_name("tv-dice");
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
    if !kept {
        die.set_attribute("data-kept", "false")?;
    }
    die.set_attribute("style", "outline:none")?;
    let image = node(document, &die, "img", "tv-die-rest", "")?;
    image.set_attribute("src", &crate::dice_roll::fallback_rest_url(clip))?;
    image.set_attribute("alt", "")?;
    image.set_attribute("style", "display:block")?;
    let figure = node(document, &die, "div", "tv-die-num", &face.to_string())?;
    figure.set_attribute("data-settled", "true")?;
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
        "left:50%;top:49%;width:30%;height:30%;font-size:calc(var(--u)*4);opacity:1!important;transform:translate(-50%,-50%)",
    )?;
    Ok(())
}

/// Takes the dice down: the roll card resolves from here. Both hosts sit under the page
/// root, so one pass takes down any shown dice wherever they were hosted. Pending number
/// timers are cancelled with their dice, so no old number can surface late.
pub(super) fn clear(client: &Rc<RefCell<Client>>) -> Result<(), JsValue> {
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
    let root = client.borrow().root.clone();
    let previous = root.query_selector_all(".tv-dice")?;
    for index in 0..previous.length() {
        if let Some(shown) = previous.item(index)
            && let Some(parent) = shown.parent_node()
            && let Ok(parent) = parent.dyn_into::<Element>()
        {
            parent.remove_child(&shown)?;
        }
    }
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

/// The element the dice are hosted in: the battlefield container while it is mounted (it
/// paints over the whole stage, so dice on the stage root would hide behind it), else the
/// flat roll panel in flow (so the dice never cover its text and never hide behind it),
/// else the stage root. The choice is [`crate::dice_roll::dice_host`] so the rule is pinned
/// by a native test; this only reads which host the page has.
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

fn die(
    document: &Document,
    parent: &Element,
    clip: usize,
    face: u8,
    kept: bool,
    natural: Option<Natural>,
    still: bool,
) -> Result<(), JsValue> {
    let settled_now = still;
    let die = node(document, parent, "div", "tv-die", "")?;
    if !kept {
        die.set_attribute("data-kept", "false")?;
    }
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
    let (center, corners, rotation, rest_ms) = LIBRARY.with(|library| {
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
    // resting frame instead of standing black behind the number.
    let reveal = Closure::wrap(Box::new(move || {
        let _shown = settled.set_attribute("data-settled", "true");
        let _styled = settled.set_attribute("style", &settled_style);
        if video.as_ref().is_some_and(|clip| clip.ready_state() < 3) {
            let _marked = landed.set_attribute("data-stalled", "true");
        }
    }) as Box<dyn FnMut()>);
    if let Some(window) = web_sys::window()
        && let Ok(id) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            reveal.as_ref().unchecked_ref(),
            rest_ms.min(2_500) as i32,
        )
    {
        LIBRARY.with(|library| {
            let mut library = library.borrow_mut();
            library.timeouts.push(id);
            library.held.push(reveal);
        });
    } else {
        let _shown = figure.set_attribute("data-settled", "true");
    }
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
