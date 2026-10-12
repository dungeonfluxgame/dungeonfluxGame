//! `BattleStage`: mounts the canvas and HUD, owns the animation-frame loop and the WebGPU
//! renderer, and is the single hard fallback signal for the caller. When WebGPU is absent, the
//! adapter or device is refused, a shader fails, or the device is lost, the stage becomes
//! [`Availability::Unavailable`] exactly once, records the reason once on the host element as
//! `data-battlefield="unavailable:<reason>"`, removes its own elements and never retries; the
//! caller then keeps (or restores) the flat combat view.

use std::{cell::RefCell, rc::Rc, time::Duration};

use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Element, HtmlCanvasElement};

use super::{
    gpu::{
        core::{self, GpuCore},
        renderer::{PendingSplats, Renderer},
        textures,
    },
    hud::Hud,
    scene_file::{BattleScene, SplatFormat, SplatLayer},
    sound::SoundEvent,
    splat::{
        ProceduralGround, SogLoader, SogMeta, SogPlanes, SplatCloud, SplatFileLoader, SplatLoader,
        SplatTransform,
    },
    stage::Stage,
};

pub use super::gpu::core::UnavailableReason;

const MAX_DEVICE_PIXEL_RATIO: f64 = 2.0;
const MAX_CANVAS_SIDE: f64 = 3840.0;
const MAX_SPLAT_FILE_BYTES: usize = 256 * 1024 * 1024;
const PROCEDURAL_SPLATS: usize = 260_000;
/// Frames after which the battlefield is shown even if some art is still loading.
const REVEAL_AFTER_FRAMES: u64 = 240;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Availability {
    Available,
    Unavailable(UnavailableReason),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StageOptions {
    pub reduced_motion: bool,
    /// Replace the scene's own splat layer with the built-in procedural ground.
    pub procedural_splats: bool,
}

struct Inner {
    host: Element,
    container: Option<Element>,
    canvas: Option<HtmlCanvasElement>,
    hud: Option<Hud>,
    stage: Stage,
    renderer: Option<Renderer>,
    availability: Availability,
    last_timestamp: Option<f64>,
    frame_handle: Option<i32>,
    disposed: bool,
    revealed: bool,
    frames: u64,
    stats_every: u64,
    /// Where the stage's sounds go, at the frame they happen. Without one they are dropped.
    sound_sink: Option<Rc<dyn Fn(SoundEvent)>>,
}

/// The animation-frame closure, shared so the loop can reschedule itself and `dispose` can drop it.
type FrameCallback = Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>>;

pub struct BattleStage {
    inner: Rc<RefCell<Inner>>,
    frame_callback: FrameCallback,
}

fn mark(host: &Element, value: &str) {
    // Setting an attribute on a connected element only fails for an invalid name; this one is fixed.
    let _ = host.set_attribute("data-battlefield", value);
}

impl BattleStage {
    /// An inert stage for a device that cannot draw the battlefield. It records the reason once on
    /// `host` and reports [`Availability::Unavailable`]; every method is a harmless no-op.
    pub fn unavailable(host: &Element, scene: BattleScene, reason: UnavailableReason) -> Self {
        mark(host, &format!("unavailable:{}", reason.key()));
        Self {
            inner: Rc::new(RefCell::new(Inner {
                host: host.clone(),
                container: None,
                canvas: None,
                hud: None,
                stage: Stage::new(scene, true),
                renderer: None,
                availability: Availability::Unavailable(reason),
                last_timestamp: None,
                frame_handle: None,
                disposed: false,
                revealed: false,
                frames: 0,
                stats_every: 60,
                sound_sink: None,
            })),
            frame_callback: Rc::new(RefCell::new(None)),
        }
    }

    /// Builds the canvas, HUD and renderer inside `host` and starts the frame loop. On any
    /// failure the returned stage is unavailable and `host` carries the reason.
    pub async fn mount(host: &Element, scene: BattleScene, options: StageOptions) -> Self {
        let Some(document) = host.owner_document() else {
            return Self::unavailable(host, scene, UnavailableReason::NotMounted);
        };
        let Ok(container) = document.create_element("div") else {
            return Self::unavailable(host, scene, UnavailableReason::NotMounted);
        };
        container.set_class_name("battlefield");
        let Ok(canvas) = document.create_element("canvas").and_then(|element| {
            element
                .dyn_into::<HtmlCanvasElement>()
                .map_err(JsValue::from)
        }) else {
            return Self::unavailable(host, scene, UnavailableReason::NotMounted);
        };
        canvas.set_class_name("bf-canvas");
        if container.append_child(&canvas).is_err() || host.append_child(&container).is_err() {
            return Self::unavailable(host, scene, UnavailableReason::NotMounted);
        }
        let gpu = match core::acquire(&canvas).await {
            Ok(gpu) => Rc::new(gpu),
            Err(reason) => {
                container.remove();
                return Self::unavailable(host, scene, reason);
            }
        };
        let pending: PendingSplats = Rc::new(RefCell::new(None));
        let renderer = match Renderer::new(gpu.clone(), &scene, pending.clone()).await {
            Ok(renderer) => renderer,
            Err((reason, message)) => {
                container.remove();
                let stage = Self::unavailable(host, scene, reason);
                let _ = host.set_attribute("data-battlefield-error", &message);
                return stage;
            }
        };
        let hud = match Hud::mount(&document, &container, &scene.title) {
            Ok(hud) => hud,
            Err(_) => {
                container.remove();
                return Self::unavailable(host, scene, UnavailableReason::NotMounted);
            }
        };
        let splat_layer = if options.procedural_splats {
            Some(SplatLayer {
                url: String::new(),
                format: SplatFormat::Procedural,
                scale: 1.0,
                offset_y: 0.0,
                rot_y_deg: 0.0,
                translate: super::math::Vec3::ZERO,
                max_splats: None,
            })
        } else {
            scene.splat.clone()
        };
        if let Some(layer) = splat_layer {
            spawn_splat_load(gpu.clone(), layer, pending);
        }
        mark(host, "active");
        let _ = host.set_attribute("data-battlefield-adapter", &gpu.adapter_description);
        let stage = Stage::new(scene, options.reduced_motion);
        let inner = Rc::new(RefCell::new(Inner {
            host: host.clone(),
            container: Some(container),
            canvas: Some(canvas),
            hud: Some(hud),
            stage,
            renderer: Some(renderer),
            availability: Availability::Available,
            last_timestamp: None,
            frame_handle: None,
            disposed: false,
            revealed: false,
            frames: 0,
            stats_every: 30,
            sound_sink: None,
        }));
        let callback: FrameCallback = Rc::new(RefCell::new(None));
        {
            let inner = inner.clone();
            let callback_for_loop = callback.clone();
            *callback.borrow_mut() = Some(Closure::wrap(Box::new(move |timestamp: f64| {
                tick(&inner, timestamp);
                schedule(&inner, &callback_for_loop);
            }) as Box<dyn FnMut(f64)>));
        }
        schedule(&inner, &callback);
        Self {
            inner,
            frame_callback: callback,
        }
    }

    pub fn availability(&self) -> Availability {
        self.inner.borrow().availability
    }

    /// Runs `operation` on the stage, or returns `None` when the battlefield is unavailable.
    pub fn with_stage<R>(&self, operation: impl FnOnce(&mut Stage) -> R) -> Option<R> {
        let mut inner = self.inner.borrow_mut();
        match inner.availability {
            Availability::Available => Some(operation(&mut inner.stage)),
            Availability::Unavailable(_) => None,
        }
    }

    /// Sends the stage's combat sounds to `sink` the moment their timeline steps happen. The sink
    /// runs inside the frame callback and must not call back into the stage.
    pub fn set_sound_sink(&self, sink: Rc<dyn Fn(SoundEvent)>) {
        self.inner.borrow_mut().sound_sink = Some(sink);
    }

    /// Stops the loop and removes everything the stage added. Safe to call more than once.
    pub fn dispose(&self) {
        let mut inner = self.inner.borrow_mut();
        teardown(&mut inner);
        inner.disposed = true;
        self.frame_callback.borrow_mut().take();
    }
}

impl Drop for BattleStage {
    fn drop(&mut self) {
        self.dispose();
    }
}

fn teardown(inner: &mut Inner) {
    if let (Some(handle), Some(window)) = (inner.frame_handle.take(), web_sys::window()) {
        let _ = window.cancel_animation_frame(handle);
    }
    if let Some(hud) = inner.hud.take() {
        hud.dispose();
    }
    inner.renderer = None;
    inner.canvas = None;
    if let Some(container) = inner.container.take() {
        container.remove();
    }
}

fn schedule(inner: &Rc<RefCell<Inner>>, callback: &FrameCallback) {
    let mut state = inner.borrow_mut();
    if state.disposed || state.availability != Availability::Available {
        return;
    }
    let (Some(window), Some(closure)) = (
        web_sys::window(),
        callback.borrow().as_ref().map(|c| c.as_ref().clone()),
    ) else {
        return;
    };
    state.frame_handle = window.request_animation_frame(closure.unchecked_ref()).ok();
}

/// Moves to unavailable once: records the reason, tears everything down, stops the loop.
fn fail(inner: &mut Inner, reason: UnavailableReason) {
    teardown(inner);
    inner.availability = Availability::Unavailable(reason);
    mark(&inner.host, &format!("unavailable:{}", reason.key()));
}

fn tick(inner: &Rc<RefCell<Inner>>, timestamp: f64) {
    let mut guard = inner.borrow_mut();
    let state = &mut *guard;
    if state.disposed || state.availability != Availability::Available {
        return;
    }
    if let Some(renderer) = &state.renderer
        && renderer.core().lost().is_some()
    {
        fail(state, UnavailableReason::DeviceLost);
        return;
    }
    let delta_ms = state
        .last_timestamp
        .map_or(16.0, |last| (timestamp - last).clamp(0.0, 100.0));
    state.last_timestamp = Some(timestamp);
    state
        .stage
        .update(Duration::from_secs_f64(delta_ms / 1000.0));
    for event in state.stage.take_sounds() {
        if let Some(sink) = &state.sound_sink {
            sink(event);
        }
    }

    let (Some(canvas), Some(renderer)) = (&state.canvas, &mut state.renderer) else {
        return;
    };
    let ratio = web_sys::window()
        .map_or(1.0, |window| window.device_pixel_ratio())
        .clamp(1.0, MAX_DEVICE_PIXEL_RATIO);
    let width = (canvas.client_width() as f64 * ratio).clamp(2.0, MAX_CANVAS_SIDE) as u32;
    let height = (canvas.client_height() as f64 * ratio).clamp(2.0, MAX_CANVAS_SIDE) as u32;
    if canvas.width() != width || canvas.height() != height {
        canvas.set_width(width);
        canvas.set_height(height);
    }
    let frame = state.stage.frame();
    renderer.render(&frame, state.stage.scene(), (width, height));
    renderer.observe_frame_ms(delta_ms as f32);
    state.frames += 1;
    // Show the battlefield once its art has arrived (or after a moment regardless), so the table
    // does not watch placeholder discs turn into figures.
    if !state.revealed
        && state.frames > 2
        && (renderer.pending_loads() == 0 || state.frames > REVEAL_AFTER_FRAMES)
    {
        state.revealed = true;
        if let Some(container) = &state.container {
            let _ = container.set_attribute("data-ready", "");
        }
    }
    if let Some(hud) = &mut state.hud {
        let aspect = width as f32 / height.max(1) as f32;
        if hud.update(&state.stage, &frame, aspect).is_err() {
            fail(state, UnavailableReason::NotMounted);
            return;
        }
    }
    if state.frames.is_multiple_of(state.stats_every) {
        let stats = renderer.stats();
        let summary = format!(
            "ms={delta_ms:.1} splats={}/{} cap={} scale={:.2} frames={}",
            stats.splats_drawn, stats.splats_loaded, stats.splat_cap, stats.scale, state.frames
        );
        let _ = state.host.set_attribute("data-battlefield-stats", &summary);
        if let Some(message) = renderer.core().first_error() {
            let _ = state.host.set_attribute("data-battlefield-error", &message);
        }
    }
}

fn spawn_splat_load(gpu: Rc<GpuCore>, layer: SplatLayer, pending: PendingSplats) {
    wasm_bindgen_futures::spawn_local(async move {
        let raw = match layer.format {
            SplatFormat::Procedural => ProceduralGround {
                seed: 1,
                splats: PROCEDURAL_SPLATS,
                half_extent_m: 16.0,
            }
            .load()
            .ok(),
            SplatFormat::Splat => {
                match textures::fetch_bytes(&layer.url, MAX_SPLAT_FILE_BYTES).await {
                    Ok(bytes) => SplatFileLoader { bytes: &bytes }.load().ok(),
                    Err(_) => None,
                }
            }
            SplatFormat::Sog => load_sog(&gpu, &layer.url).await,
        };
        let Some(raw) = raw else {
            // The splat scene is optional: without it the painted ground stays.
            return;
        };
        let transform = SplatTransform {
            scale: layer.scale,
            offset_y: layer.offset_y,
            rot_y_deg: layer.rot_y_deg,
            translate: layer.translate,
        };
        let (cloud, _dropped) = SplatCloud::bake(&raw, transform);
        *pending.borrow_mut() = Some(cloud);
    });
}

/// Fetches `<base>/meta.json` and its five planes, decodes each plane exactly, and loads them.
async fn load_sog(gpu: &GpuCore, meta_url: &str) -> Option<Vec<super::splat::RawSplat>> {
    let meta_text = textures::fetch_text(meta_url, 1024 * 1024).await.ok()?;
    let meta = SogMeta::parse(&meta_text).ok()?;
    let base = meta_url.rsplit_once('/').map_or("", |(base, _)| base);
    let plane = |name: &str| format!("{base}/{name}");
    let means_l = textures::read_exact_rgba(gpu, &plane(&meta.means_files[0]))
        .await
        .ok()?;
    let means_u = textures::read_exact_rgba(gpu, &plane(&meta.means_files[1]))
        .await
        .ok()?;
    let quats = textures::read_exact_rgba(gpu, &plane(&meta.quats_file))
        .await
        .ok()?;
    let scales = textures::read_exact_rgba(gpu, &plane(&meta.scales_file))
        .await
        .ok()?;
    let sh0 = textures::read_exact_rgba(gpu, &plane(&meta.sh0_file))
        .await
        .ok()?;
    SogLoader {
        meta: &meta,
        planes: SogPlanes {
            means_l: &means_l.0,
            means_u: &means_u.0,
            quats: &quats.0,
            scales: &scales.0,
            sh0: &sh0.0,
        },
    }
    .load()
    .ok()
}
