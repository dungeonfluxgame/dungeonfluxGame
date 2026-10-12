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
        renderer::{DEFAULT_SPLAT_CAPACITY, Renderer, SplatSetup},
        splat_engine::{LoadState, Shared, SharedSplats},
        splat_stream::{self, StreamConfig, next_frame},
        textures,
    },
    hud::Hud,
    pacing::{BattleFps, FramePacer, Pace},
    scene_file::{BackdropLayer, BattleScene, GroundLayer, SplatFormat, SplatGrade, SplatLayer},
    sound::SoundEvent,
    speech_bubble::SpeechBubble,
    splash::{self, Gate, Phase, Readiness},
    splat::{
        ProceduralGround, RawSplat, SogLoader, SogMeta, SogPlanes, SplatFileLoader, SplatLoader,
        SplatTransform, pack_records,
    },
    stage::{ImpactEvent, Stage},
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

#[derive(Clone, Debug, Default)]
pub struct StageOptions {
    /// Replace the scene's own splat layer with the built-in procedural ground.
    pub procedural_splats: bool,
    /// The painted ground and backdrop to show instead when the scene's splats cannot be drawn
    /// (files missing, decode failure, or a frame budget that cannot be met). Without it the
    /// scene's own painted layers stay.
    pub fallback_layers: Option<(GroundLayer, BackdropLayer)>,
    /// Measurement overrides: freeze the frame ladder at these values.
    pub fixed_quality: Option<FixedQuality>,
    /// The presentation rate: the battlefield never draws faster than this.
    pub fps: BattleFps,
    /// Battle feel (shake, hit-stop, sway, blur, particles); off only for measurements.
    pub feel_off: bool,
    /// Runs the stage's clock at this share of real time (a review harness captures slow motion
    /// strips with it); `None` is real time.
    pub slow: Option<f32>,
    /// Which tier of a streamed scan to draw as the finest: 0 is the largest tier the scene
    /// authors for the scan, 1 the next one down (the default of the live game), and so on, never
    /// coarser than the scene's coarsest tier.
    pub splat_lod: u32,
}

/// Quality held constant for benchmarks (`#battle-preview=...&fixed&budget=..&minc=..&rscale=..`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedQuality {
    pub splat_budget: Option<u32>,
    pub min_contribution: Option<f32>,
    pub render_scale: Option<f32>,
    pub splat_scale: Option<f32>,
    pub debug_switches: u32,
}

struct Inner {
    host: Element,
    container: Option<Element>,
    canvas: Option<HtmlCanvasElement>,
    hud: Option<Hud>,
    speech: Option<SpeechBubble>,
    stage: Stage,
    renderer: Option<Renderer>,
    availability: Availability,
    /// Decides which animation-frame calls present a frame (the fixed 30 or 24 fps cadence).
    pacer: FramePacer,
    /// Share of real time the stage clock runs at (1 in play).
    time_scale: f64,
    frame_handle: Option<i32>,
    disposed: bool,
    revealed: bool,
    /// The loading splash over the battlefield, until the scene is resolved and revealed.
    splash: Option<SplashDom>,
    gate: Gate,
    /// Whether the stage was busy at the last frame, for the `data-battlefield-busy` marker.
    last_busy: bool,
    /// Chunks resident, chunks wanted and whether the scan is complete, for the splash.
    scan_progress: (u32, u32, bool),
    frames: u64,
    stats_every: u64,
    fallback: Option<(GroundLayer, BackdropLayer)>,
    fallback_applied: bool,
    /// Where the stage's sounds go, at the frame they happen. Without one they are dropped.
    sound_sink: Option<Rc<dyn Fn(SoundEvent)>>,
    /// Where blows that landed go, with their intensity: the one hook the HUD's own hit-shake
    /// subscribes to. Without one they are dropped.
    impact_sink: Option<Rc<dyn Fn(ImpactEvent)>>,
}

/// The loading splash: the place, the mark and a bar driven by the real chunk count.
struct SplashDom {
    root: Element,
    bar: Element,
    note: Element,
    note_text: String,
}

const MARK_SVG: &str = "<svg viewBox=\"0 0 64 64\" aria-hidden=\"true\"><path d=\"M32 3 58 18v28L32 61 6 46V18Z\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.2\"/><path d=\"M32 3 19 40h26ZM6 18l13 22M58 18 45 40M19 40l13 21 13-21\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.4\" stroke-linejoin=\"round\"/></svg>";

impl SplashDom {
    fn mount(document: &web_sys::Document, host: &Element, place: &str) -> Result<Self, JsValue> {
        let root = document.create_element("div")?;
        root.set_class_name("bf-splash");
        root.set_attribute("role", "status")?;
        let child =
            |parent: &Element, tag: &str, class: &str, text: &str| -> Result<Element, JsValue> {
                let node = document.create_element(tag)?;
                node.set_class_name(class);
                if !text.is_empty() {
                    node.set_text_content(Some(text));
                }
                parent.append_child(&node)?;
                Ok(node)
            };
        let mark = child(&root, "div", "bf-splash-mark", "")?;
        mark.set_inner_html(MARK_SVG);
        child(&root, "div", "bf-splash-brand", "DungeonFlux")?;
        child(&root, "div", "bf-splash-place", place)?;
        let track = child(&root, "div", "bf-splash-track", "")?;
        let bar = child(&track, "i", "bf-splash-bar", "")?;
        let note_text = splash::label(0, 0, false);
        let note = child(&root, "div", "bf-splash-note", &note_text)?;
        host.append_child(&root)?;
        Ok(Self {
            root,
            bar,
            note,
            note_text,
        })
    }

    fn show(&mut self, loaded: u32, wanted: u32, complete: bool) {
        let fraction = splash::progress(loaded, wanted, complete);
        let _ = self
            .bar
            .set_attribute("style", &format!("transform:scaleX({fraction:.4})"));
        let text = splash::label(loaded, wanted, complete);
        if text != self.note_text {
            self.note.set_text_content(Some(&text));
            self.note_text = text;
        }
    }
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
                speech: None,
                stage: Stage::new(scene),
                renderer: None,
                availability: Availability::Unavailable(reason),
                pacer: FramePacer::new(BattleFps::DEFAULT),
                time_scale: 1.0,
                frame_handle: None,
                disposed: false,
                revealed: false,
                splash: None,
                gate: Gate::new(0.0),
                last_busy: false,
                scan_progress: (0, 0, false),
                frames: 0,
                stats_every: 60,
                fallback: None,
                fallback_applied: false,
                sound_sink: None,
                impact_sink: None,
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
        let splat_layer = if options.procedural_splats {
            Some(SplatLayer {
                url: String::new(),
                format: SplatFormat::Procedural,
                scale: 1.0,
                offset_y: 0.0,
                rot_y_deg: 0.0,
                translate: super::math::Vec3::ZERO,
                max_splats: None,
                orient: super::math::Quat::IDENTITY,
                levels: None,
                detail_radius_m: None,
                grade: SplatGrade::NEUTRAL,
                depth_bias_m: 0.15,
                clip: None,
            })
        } else {
            scene.splat.clone()
        };
        let splat_setup = splat_layer.as_ref().map(|layer| SplatSetup {
            shared: Rc::new(RefCell::new(Shared::new(
                layer.max_splats.unwrap_or(DEFAULT_SPLAT_CAPACITY),
            ))),
            layer: layer.clone(),
        });
        let splat_shared = splat_setup.as_ref().map(|setup| setup.shared.clone());
        let renderer = match Renderer::new(gpu.clone(), &scene, splat_setup).await {
            Ok(renderer) => renderer,
            Err((reason, message)) => {
                container.remove();
                let stage = Self::unavailable(host, scene, reason);
                let _ = host.set_attribute("data-battlefield-error", &message);
                return stage;
            }
        };
        let mut renderer = renderer;
        if let Some(fixed) = options.fixed_quality {
            renderer.debug_switches = fixed.debug_switches;
            renderer.freeze_quality(
                fixed.splat_budget,
                fixed.min_contribution,
                fixed.render_scale,
                fixed.splat_scale,
            );
        }
        let hud = match Hud::mount(&document, &container, &scene.title) {
            Ok(hud) => hud,
            Err(_) => {
                container.remove();
                return Self::unavailable(host, scene, UnavailableReason::NotMounted);
            }
        };
        if let (Some(layer), Some(shared)) = (splat_layer, splat_shared) {
            spawn_splat_load(
                gpu.clone(),
                layer,
                shared,
                renderer.splat_chunk_layout(),
                scene.grid.centre(),
                scene.grid.cols() as f32 * scene.grid.cell_m(),
                scene.grid.rows() as f32 * scene.grid.cell_m(),
                options.splat_lod,
            );
        }
        mark(host, "active");
        let _ = host.set_attribute("data-battlefield-adapter", &gpu.adapter_description);
        let mut stage = Stage::new(scene);
        if options.feel_off {
            stage.set_feel_enabled(false);
        }
        let splash = SplashDom::mount(&document, host, &stage.scene().title).ok();
        let speech = SpeechBubble::mount(&document, &container).ok();
        let inner = Rc::new(RefCell::new(Inner {
            host: host.clone(),
            container: Some(container),
            canvas: Some(canvas),
            hud: Some(hud),
            speech,
            stage,
            renderer: Some(renderer),
            availability: Availability::Available,
            pacer: FramePacer::new(options.fps),
            time_scale: options
                .slow
                .map_or(1.0, |share| f64::from(share.clamp(0.02, 1.0))),
            frame_handle: None,
            disposed: false,
            revealed: false,
            splash,
            gate: Gate::new(now_ms()),
            last_busy: false,
            scan_progress: (0, 0, false),
            frames: 0,
            stats_every: 30,
            fallback: options.fallback_layers,
            fallback_applied: false,
            sound_sink: None,
            impact_sink: None,
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

    /// Subscribes to blows landing: each [`ImpactEvent`] says who was hit and how heavily (0 to 1),
    /// at the frame it happens. The HUD's own hit-shake hangs off this; the camera shake is
    /// already done by the stage.
    pub fn set_impact_sink(&self, sink: Rc<dyn Fn(ImpactEvent)>) {
        self.inner.borrow_mut().impact_sink = Some(sink);
    }

    /// Sets the tempo cue's intensity (0 calm to 1000 peak) that the camera's sway follows.
    pub fn set_tempo_intensity(&self, permille: u16) {
        self.inner.borrow_mut().stage.set_tempo_intensity(permille);
    }

    /// Whether the scene is resolved: the scan at the chosen level fully drawn, its art arrived
    /// and the figures placed. Until then the loading splash holds and nothing of the fight plays.
    pub fn is_resolved(&self) -> bool {
        self.inner.borrow().gate.resolved()
    }

    /// How far the scan has got: chunks resident, chunks wanted and whether it is complete.
    pub fn scan_progress(&self) -> (u32, u32, bool) {
        self.inner.borrow().scan_progress
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
    if let Some(bubble) = inner.speech.take() {
        bubble.dispose();
    }
    if let Some(splash) = inner.splash.take() {
        splash.root.remove();
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

/// Milliseconds on the page's high-resolution clock; zero when there is no `performance`.
fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0.0, |performance| performance.now())
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
    // Ticks between presented frames do no work at all: no update, no GPU submission.
    let Pace::Present { elapsed_ms } = state.pacer.tick(timestamp) else {
        return;
    };
    let tick_started_ms = now_ms();
    // The simulation advances by real time since the previous presented frame, so speeds are
    // unchanged by the cap.
    let delta_ms = elapsed_ms.clamp(0.0, 100.0) * state.time_scale;
    // Nothing of the fight plays behind the loading splash: the stage clock waits for it.
    if state.gate.resolved() {
        state
            .stage
            .update(Duration::from_secs_f64(delta_ms / 1000.0));
    }
    for event in state.stage.take_sounds() {
        if let Some(sink) = &state.sound_sink {
            sink(event);
        }
    }
    for event in state.stage.take_impacts() {
        if let Some(sink) = &state.impact_sink {
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
    // Where the figures stand, written with the periodic stats (for harnesses and diagnostics).
    let figures: Vec<String> = if (state.frames + 1).is_multiple_of(state.stats_every) {
        frame
            .figures
            .iter()
            .map(|figure| {
                format!(
                    "{}@{:.2},{:.2},{:.2}",
                    figure.name, figure.position.x, figure.position.y, figure.position.z
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    renderer.render(&frame, state.stage.scene(), (width, height));
    // What the frame cost, not how long the cap lets it take: the quality ladder compares it with
    // the cap's period. Without GPU timing, only an interval well over the period shows a miss.
    let period_ms = state.pacer.target().period_ms() as f32;
    let gpu_ms = renderer.stats().gpu_ms;
    let cpu_ms = (now_ms() - tick_started_ms) as f32;
    let cost_ms = if gpu_ms > 0.0 {
        gpu_ms.max(cpu_ms)
    } else if delta_ms as f32 > period_ms * 1.25 {
        delta_ms as f32
    } else {
        cpu_ms
    };
    renderer.observe_frame_cost(cost_ms, period_ms);
    state.frames += 1;
    // The splash holds until the scan at the chosen level is resolved, the art has arrived and the
    // figures stand on their cells; then one transition reveals the finished scene.
    let (scan, loaded, wanted) = match renderer.splat_shared() {
        Some(splat) => {
            let shared = splat.borrow();
            let finished = matches!(shared.state, LoadState::Complete | LoadState::Failed);
            (
                finished || renderer.splat_unusable(),
                shared.stats.chunks_loaded,
                shared.stats.chunks_wanted,
            )
        }
        None => (true, 0, 0),
    };
    let complete = scan && wanted > 0;
    state.scan_progress = (loaded, wanted, complete);
    let phase = state.gate.observe(
        now_ms(),
        Readiness {
            scan,
            art: renderer.pending_loads() == 0 || state.frames > REVEAL_AFTER_FRAMES,
            figures: state.stage.tokens().next().is_some(),
        },
    );
    if let Some(dom) = &mut state.splash {
        dom.show(loaded, wanted, scan);
        match phase {
            Phase::Holding => {}
            Phase::Revealing => {
                let _ = dom.root.set_attribute("data-leaving", "");
            }
            Phase::Gone => {
                dom.root.remove();
            }
        }
        if phase == Phase::Gone {
            state.splash = None;
        }
    }
    if phase != Phase::Holding && !state.revealed {
        state.revealed = true;
        if let Some(container) = &state.container {
            let _ = container.set_attribute("data-ready", "");
        }
    }
    // A harness (and the display's battle machine, through `is_busy`) reads when the stage is
    // playing something, including the held beat after the deciding blow.
    let busy = state.stage.is_busy();
    if busy != state.last_busy {
        state.last_busy = busy;
        let _ = state
            .host
            .set_attribute("data-battlefield-busy", if busy { "1" } else { "0" });
    }
    if state.frames.is_multiple_of(state.stats_every) {
        let label = match phase {
            Phase::Holding => "holding",
            Phase::Revealing => "revealing",
            Phase::Gone => "gone",
        };
        let _ = state.host.set_attribute("data-battlefield-splash", label);
    }
    if let Some(hud) = &mut state.hud {
        let aspect = width as f32 / height.max(1) as f32;
        if hud.update(&state.stage, &frame, aspect).is_err() {
            fail(state, UnavailableReason::NotMounted);
            return;
        }
    }
    if let (Some(bubble), Some(container)) = (&mut state.speech, &state.container) {
        let aspect = width as f32 / height.max(1) as f32;
        // A bubble that cannot be drawn is cosmetic; the fight carries on without it.
        let _ = bubble.update(container, &state.stage, &frame, aspect);
    }
    if !state.fallback_applied
        && let Some((ground, backdrop)) = state.fallback.clone()
        && renderer.splat_unusable()
    {
        state.fallback_applied = true;
        renderer.set_painted(&ground.stem, &backdrop.stem);
        state.stage.replace_painted_layers(ground, backdrop);
        let _ = state
            .host
            .set_attribute("data-battlefield-fallback", "painted");
    }
    if state.frames.is_multiple_of(state.stats_every) {
        let _ = state
            .host
            .set_attribute("data-battlefield-figures", &figures.join(";"));
        let stats = renderer.stats();
        let summary = format!(
            "ms={delta_ms:.1} fps_target={} fps_presented={:.1} fps_divisor={} display_hz={:.0} cost_ms={cost_ms:.1} gpu_ms={:.1} sort_ms={:.2} splat_ms={:.2} overlay_ms={:.2} post_ms={:.2} visible={} planned={} budget={} min_contribution={:.2} scale={:.2} splat_scale={:.2} frames={}",
            state.pacer.target().hz(),
            state.pacer.presented_fps(),
            state.pacer.divisor(),
            1000.0 / state.pacer.display_interval_ms(),
            stats.gpu_ms,
            stats.sort_ms,
            stats.splat_ms,
            stats.overlay_ms,
            stats.post_ms,
            stats.splats_visible,
            stats.splats_planned,
            stats.budget,
            stats.min_contribution,
            stats.scale,
            stats.splat_scale,
            state.frames
        );
        let _ = state.host.set_attribute("data-battlefield-stats", &summary);
        if let Some(splat) = renderer.splat_shared() {
            let shared = splat.borrow();
            let label = match (stats.given_up, shared.state) {
                (true, _) => "given-up",
                (_, LoadState::Idle) => "idle",
                (_, LoadState::Loading) => "loading",
                (_, LoadState::Showing) => "showing",
                (_, LoadState::Complete) => "complete",
                (_, LoadState::Failed) => "failed",
            };
            let _ = state.host.set_attribute("data-battlefield-splat", label);
            let load = &shared.stats;
            let timing = |value: Option<f64>| value.map_or("-".to_owned(), |ms| format!("{ms:.0}"));
            let _ = state.host.set_attribute(
                "data-battlefield-load",
                &format!(
                    "first_ms={} full_ms={} chunks={}/{}",
                    timing(load.first_draw_ms),
                    timing(load.full_detail_ms),
                    load.chunks_loaded,
                    load.chunks_wanted
                ),
            );
        }
        if let Some(message) = renderer.core().first_error() {
            let _ = state.host.set_attribute("data-battlefield-error", &message);
        }
    }
}

/// Starts loading the scene's splats. A streamed scan hands everything to the streaming task; the
/// small formats are decoded here and uploaded once as packed records, in slices so the page
/// keeps drawing.
fn spawn_splat_load(
    gpu: Rc<GpuCore>,
    layer: SplatLayer,
    shared: SharedSplats,
    chunk_layout: Option<JsValue>,
    grid_centre: super::math::Vec3,
    grid_width_m: f32,
    grid_depth_m: f32,
    splat_lod: u32,
) {
    wasm_bindgen_futures::spawn_local(async move {
        let transform = SplatTransform {
            scale: layer.scale,
            offset_y: layer.offset_y,
            rot_y_deg: layer.rot_y_deg,
            translate: layer.translate,
            orient: layer.orient,
        };
        if layer.format == SplatFormat::Lod {
            let Some(chunk_layout) = chunk_layout else {
                shared.borrow_mut().state = LoadState::Failed;
                return;
            };
            // Finest detail covers the grid and a margin; the scene file may widen it.
            let derived =
                0.5 * (grid_width_m * grid_width_m + grid_depth_m * grid_depth_m).sqrt() + 3.0;
            let (authored_finest, coarsest) = layer.levels.unwrap_or((0, 0));
            let finest = authored_finest.saturating_add(splat_lod).min(coarsest);
            splat_stream::stream(
                gpu,
                shared,
                chunk_layout,
                StreamConfig {
                    index_url: layer.url.clone(),
                    levels: (finest, coarsest),
                    transform,
                    focus_world: grid_centre,
                    detail_radius_m: layer.detail_radius_m.unwrap_or(derived),
                },
            )
            .await;
            return;
        }
        shared.borrow_mut().state = LoadState::Loading;
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
            SplatFormat::Lod => None,
        };
        let Some(raw) = raw else {
            // The splat scene is optional: without it the painted ground stays.
            shared.borrow_mut().state = LoadState::Failed;
            return;
        };
        let words = pack_in_slices(&raw, transform).await;
        let mut state = shared.borrow_mut();
        state.stats.splats_planned = (words.len() / super::splat::RECORD_WORDS) as u32;
        state.records = Some(words);
        state.state = LoadState::Complete;
    });
}

/// Packs `raw` into GPU records a slice at a time, yielding a frame between slices.
async fn pack_in_slices(raw: &[RawSplat], transform: SplatTransform) -> Vec<u32> {
    const SLICE: usize = 60_000;
    let mut words = Vec::with_capacity(raw.len() * super::splat::RECORD_WORDS);
    for slice in raw.chunks(SLICE) {
        let (part, _dropped) = pack_records(slice, transform);
        words.extend_from_slice(&part);
        next_frame().await;
    }
    words
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
