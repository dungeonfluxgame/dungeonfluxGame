//! The WebGPU renderer: one scene pass (backdrop, ground or splats, grid decals, figures) into an
//! off-screen colour target, then a post pass to the canvas (grade, tilt-shift, vignette, flash).
//! It draws what [`Stage::frame`](crate::battle::Stage::frame) describes and owns no game state.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use wasm_bindgen::JsValue;

use super::{
    core::{COLOUR_TARGET_USAGE, GpuCore, UnavailableReason},
    shaders,
    splat_engine::{FrameParams, LoadState, SharedSplats, SplatEngine},
    sys::{
        BUFFER_COPY_DST, BUFFER_COPY_SRC, BUFFER_MAP_READ, BUFFER_QUERY_RESOLVE, BUFFER_STORAGE,
        BUFFER_UNIFORM, BUFFER_VERTEX, Descriptor, GpuBindGroup, GpuBuffer, GpuQuerySet,
        GpuRenderPass, GpuRenderPipeline, GpuTexture, array, float_array,
    },
    textures::{self, ImageTexture},
};
use crate::battle::{
    director::FlashTone,
    grid::Grid,
    math::Vec3,
    model::{HighlightKind, Side},
    scene_file::{BattleScene, SplatLayer},
    stage::FrameView,
};

const GENERATED: &str = "/assets/ui/generated";
const DEPTH_FORMAT: &str = "depth32float";
const COLOUR_FORMAT: &str = "rgba8unorm";
const DECAL_STRIDE: usize = 12;
/// Vertices per decal: a 6 by 6 patch of quads (see the decal shader).
const DECAL_VERTICES: u32 = 216;
/// How far toward the camera, along the line of sight, tiles on a scan are lifted for the depth test.
const DECAL_LIFT_M: f32 = 2.0;
const FIGURE_STRIDE: usize = 20;
const MAX_DECALS: usize = 768;
const MAX_FIGURES: usize = 16;
const NEAR: f32 = 0.1;
const FAR: f32 = 150.0;
/// Splats a scene may hold when its file does not say.
pub const DEFAULT_SPLAT_CAPACITY: u32 = 2_500_000;
const MIN_SPLAT_BUDGET: u32 = 250_000;
const MIN_SPLAT_SCALE: f32 = 0.5;
/// Projected splats whose opacity times area (pixels) is below this are skipped.
const BASE_MIN_CONTRIBUTION: f32 = 0.45;
const MAX_MIN_CONTRIBUTION: f32 = 6.0;
/// Frame time above which the ladder steps down, and below which it steps back up.
const SLOW_FRAME_MS: f32 = 21.0;
const FAST_FRAME_MS: f32 = 15.6;
/// Frames of evidence before the ladder moves: down is quick, up is patient.
const STEP_DOWN_FRAMES: u32 = 24;
const STEP_UP_FRAMES: u32 = 240;
/// Splats are given up for the painted ground when the ladder is at its floor and frames still
/// take this long for this many frames.
const GIVE_UP_FRAME_MS: f32 = 45.0;
const GIVE_UP_FRAMES: u32 = 240;

enum Slot {
    Loading,
    Ready(Rc<ImageTexture>),
    Failed,
}

type Cache = Rc<RefCell<HashMap<String, Slot>>>;

/// A splat scene to prepare the renderer for: the state the loader fills and the scene's layer.
pub struct SplatSetup {
    pub shared: SharedSplats,
    pub layer: SplatLayer,
}

struct Pipelines {
    backdrop: GpuRenderPipeline,
    ground: GpuRenderPipeline,
    decal: GpuRenderPipeline,
    figure: GpuRenderPipeline,
    post: GpuRenderPipeline,
}

struct Targets {
    width: u32,
    height: u32,
    scene: GpuTexture,
    depth: GpuTexture,
    post_group: GpuBindGroup,
}

struct SplatScene {
    engine: SplatEngine,
    layer: SplatLayer,
    /// Set when the ladder has run out of ways to go faster: the painted ground takes over.
    given_up: bool,
}

/// Cost knobs the frame-time ladder turns down on weak devices and back up with headroom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quality {
    /// Resolution the scan is drawn at, as a share of the scene's: splat fill rate is what costs
    /// time, and a scan is soft enough to be drawn smaller and filtered up.
    pub splat_scale: f32,
    /// Splats the streamed scan may plan (the level-of-detail budget).
    pub splat_budget: u32,
    /// Skips projected splats whose opacity times area in pixels is below this.
    pub min_contribution: f32,
    pub render_scale: f32,
    pub tilt_shift: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    /// Splats in the plan (before culling) and splats that survived culling in the last
    /// measured frame.
    pub splats_planned: u32,
    pub splats_visible: u32,
    pub budget: u32,
    pub min_contribution: f32,
    pub scale: f32,
    pub splat_scale: f32,
    /// Project, sort and decode time on the GPU, and the whole frame's submit-to-done time.
    pub sort_ms: f32,
    pub splat_ms: f32,
    pub overlay_ms: f32,
    pub post_ms: f32,
    pub gpu_ms: f32,
    pub splat_state: Option<LoadState>,
    pub given_up: bool,
}

/// Results of the periodic GPU timing, written by completion callbacks.
#[derive(Default)]
struct Measured {
    /// Decode excluded: project, scan and scatter of the sort.
    sort_ms: f32,
    splat_ms: f32,
    overlay_ms: f32,
    post_ms: f32,
    gpu_ms: f32,
    visible: u32,
}

/// Pass timing through timestamp queries: eight stamps, two per measured pass.
struct Timing {
    set: GpuQuerySet,
    resolve: GpuBuffer,
    read: GpuBuffer,
}

pub struct Renderer {
    core: Rc<GpuCore>,
    pipelines: Pipelines,
    frame_buffer: GpuBuffer,
    frame_group: GpuBindGroup,
    terrain_buffer: GpuBuffer,
    terrain_group: GpuBindGroup,
    post_buffer: GpuBuffer,
    decal_buffer: GpuBuffer,
    figure_buffer: GpuBuffer,
    targets: Option<Targets>,
    cache: Cache,
    backdrop_stem: String,
    ground_stem: String,
    dark_ground: ImageTexture,
    clear_pixel: Rc<ImageTexture>,
    splat: Option<SplatScene>,
    measured: Rc<RefCell<Measured>>,
    timing: Option<Timing>,
    count_readback: GpuBuffer,
    readback_busy: Rc<std::cell::Cell<bool>>,
    pub quality: Quality,
    frame_ms_average: f32,
    slow_frames: u32,
    fast_frames: u32,
    floor_frames: u32,
    ladder_frozen: bool,
    /// Diagnostic switches for benchmarks and evidence (preview `&dbg=N`): 1 the scan writes no
    /// depth (shows what occlusion hides), 2 skip the splat draw, 4 skip the post pass.
    pub debug_switches: u32,
    spot: (f32, f32, f32),
    last_clock_s: f32,
    frames_seen: u32,
    stats: RenderStats,
}

struct PipelineSpec<'a> {
    wgsl: &'a str,
    layout: &'a JsValue,
    buffers: Vec<JsValue>,
    topology: &'a str,
    /// Depth write and compare function; `None` for a pass without a depth buffer.
    depth: Option<(bool, &'a str)>,
    target_format: &'a str,
    blend: bool,
}

fn pipeline(core: &GpuCore, spec: PipelineSpec<'_>) -> GpuRenderPipeline {
    let PipelineSpec {
        wgsl,
        layout,
        buffers,
        topology,
        depth,
        target_format,
        blend,
    } = spec;
    let module = core
        .device
        .create_shader_module(&Descriptor::new().with("code", wgsl).build());
    let component = || {
        Descriptor::new()
            .with("srcFactor", "one")
            .with("dstFactor", "one-minus-src-alpha")
            .with("operation", "add")
            .build()
    };
    let mut target = Descriptor::new().with("format", target_format);
    if blend {
        target = target.with(
            "blend",
            Descriptor::new()
                .with("color", component())
                .with("alpha", component())
                .build(),
        );
    }
    let mut descriptor = Descriptor::new()
        .with("layout", layout.clone())
        .with(
            "vertex",
            Descriptor::new()
                .with("module", module.clone())
                .with("entryPoint", "vs")
                .with("buffers", array(buffers))
                .build(),
        )
        .with(
            "fragment",
            Descriptor::new()
                .with("module", module)
                .with("entryPoint", "fs")
                .with("targets", array([target.build()]))
                .build(),
        )
        .with(
            "primitive",
            Descriptor::new()
                .with("topology", topology)
                .with("cullMode", "none")
                .build(),
        );
    if let Some((write, compare)) = depth {
        descriptor = descriptor.with(
            "depthStencil",
            Descriptor::new()
                .with("format", DEPTH_FORMAT)
                .with("depthWriteEnabled", write)
                .with("depthCompare", compare)
                .build(),
        );
    }
    core.device.create_render_pipeline(&descriptor.build())
}

/// The ground height field as the decal shader reads it; a flat two-by-two field when the scene
/// has no terrain.
fn terrain_words(scene: &BattleScene) -> Vec<f32> {
    match scene.grid.terrain() {
        Some(field) => {
            let mut words = vec![
                field.origin()[0],
                field.origin()[1],
                field.step_m(),
                field.cols() as f32,
                field.rows() as f32,
                0.0,
                0.0,
                0.0,
            ];
            words.extend_from_slice(field.heights());
            words
        }
        None => vec![0.0, 0.0, 1.0, 2.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    }
}

fn instance_layout(stride_floats: usize, attributes: usize) -> JsValue {
    let attributes = (0..attributes).map(|index| {
        Descriptor::new()
            .with("shaderLocation", index as u32)
            .with("offset", (index * 16) as u32)
            .with("format", "float32x4")
            .build()
    });
    Descriptor::new()
        .with("arrayStride", (stride_floats * 4) as u32)
        .with("stepMode", "instance")
        .with("attributes", array(attributes))
        .build()
}

fn buffer(core: &GpuCore, size: usize, usage: u32) -> GpuBuffer {
    core.device.create_buffer(
        &Descriptor::new()
            .with("size", size as f64)
            .with("usage", usage)
            .build(),
    )
}

fn entry_buffer(binding: u32, buffer: &GpuBuffer) -> JsValue {
    Descriptor::new()
        .with("binding", binding)
        .with(
            "resource",
            Descriptor::new().with("buffer", buffer.clone()).build(),
        )
        .build()
}

fn bind_group(core: &GpuCore, layout: &JsValue, entries: Vec<JsValue>) -> GpuBindGroup {
    core.device.create_bind_group(
        &Descriptor::new()
            .with("layout", layout.clone())
            .with("entries", array(entries))
            .build(),
    )
}

impl Renderer {
    /// Builds every pipeline and buffer. A shader or pipeline validation error is caught with an
    /// error scope and reported as [`UnavailableReason::ShaderError`].
    pub async fn new(
        core: Rc<GpuCore>,
        scene: &BattleScene,
        splat_setup: Option<SplatSetup>,
    ) -> Result<Self, (UnavailableReason, String)> {
        core.device.push_error_scope("validation");
        let frame_layout = core.pipeline_layout(&[&core.layout_frame]);
        let texture_layout = core.pipeline_layout(&[&core.layout_frame, &core.layout_texture]);
        let terrain_layout = core.device.create_bind_group_layout(
            &Descriptor::new()
                .with(
                    "entries",
                    array([Descriptor::new()
                        .with("binding", 0)
                        .with("visibility", super::sys::SHADER_VERTEX)
                        .with(
                            "buffer",
                            Descriptor::new().with("type", "read-only-storage").build(),
                        )
                        .build()]),
                )
                .build(),
        );
        let decal_layout = core.pipeline_layout(&[&core.layout_frame, &terrain_layout]);
        let post_layout = core.pipeline_layout(&[&core.layout_post]);
        let _ = &frame_layout;
        let pipelines = Pipelines {
            backdrop: pipeline(
                &core,
                PipelineSpec {
                    wgsl: &shaders::backdrop(),
                    layout: &texture_layout,
                    buffers: vec![],
                    topology: "triangle-list",
                    depth: Some((true, "always")),
                    target_format: COLOUR_FORMAT,
                    blend: false,
                },
            ),
            ground: pipeline(
                &core,
                PipelineSpec {
                    wgsl: &shaders::ground(),
                    layout: &texture_layout,
                    buffers: vec![],
                    topology: "triangle-list",
                    depth: Some((true, "less-equal")),
                    target_format: COLOUR_FORMAT,
                    blend: true,
                },
            ),
            decal: pipeline(
                &core,
                PipelineSpec {
                    wgsl: &shaders::decal(),
                    layout: &decal_layout,
                    buffers: vec![instance_layout(DECAL_STRIDE, 3)],
                    topology: "triangle-list",
                    depth: Some((false, "less-equal")),
                    target_format: COLOUR_FORMAT,
                    blend: true,
                },
            ),
            figure: pipeline(
                &core,
                PipelineSpec {
                    wgsl: &shaders::figure(),
                    layout: &texture_layout,
                    buffers: vec![instance_layout(FIGURE_STRIDE, 5)],
                    topology: "triangle-list",
                    depth: Some((false, "less-equal")),
                    target_format: COLOUR_FORMAT,
                    blend: true,
                },
            ),
            post: pipeline(
                &core,
                PipelineSpec {
                    wgsl: &shaders::post(),
                    layout: &post_layout,
                    buffers: vec![],
                    topology: "triangle-list",
                    depth: None,
                    target_format: core.format,
                    blend: false,
                },
            ),
        };
        let frame_buffer = buffer(&core, 256, BUFFER_UNIFORM | BUFFER_COPY_DST);
        let post_buffer = buffer(&core, 128, BUFFER_UNIFORM | BUFFER_COPY_DST);
        let decal_buffer = buffer(
            &core,
            MAX_DECALS * DECAL_STRIDE * 4,
            BUFFER_VERTEX | BUFFER_COPY_DST,
        );
        let figure_buffer = buffer(
            &core,
            MAX_FIGURES * FIGURE_STRIDE * 4,
            BUFFER_VERTEX | BUFFER_COPY_DST,
        );
        let terrain_data = terrain_words(scene);
        let terrain_buffer = buffer(
            &core,
            terrain_data.len() * 4,
            BUFFER_STORAGE | BUFFER_COPY_DST,
        );
        core.queue
            .write_buffer(&terrain_buffer, 0.0, &float_array(&terrain_data));
        let terrain_group = bind_group(
            &core,
            &terrain_layout,
            vec![entry_buffer(0, &terrain_buffer)],
        );
        let frame_group = bind_group(
            &core,
            &core.layout_frame,
            vec![entry_buffer(0, &frame_buffer)],
        );
        let dark_ground = textures::solid(&core, [18, 17, 20, 255]);
        let clear_pixel = Rc::new(textures::solid(&core, [255, 255, 255, 255]));
        let error = wasm_bindgen_futures::JsFuture::from(core.device.pop_error_scope())
            .await
            .unwrap_or(JsValue::NULL);
        if !error.is_null() && !error.is_undefined() {
            let message = js_sys::Reflect::get(&error, &JsValue::from_str("message"))
                .ok()
                .and_then(|message| message.as_string())
                .unwrap_or_default();
            return Err((UnavailableReason::ShaderError, message));
        }
        let splat = match splat_setup {
            Some(setup) => {
                let capacity = setup
                    .layer
                    .max_splats
                    .unwrap_or(DEFAULT_SPLAT_CAPACITY)
                    .clamp(1_000, crate::battle::splat::MAX_SPLATS as u32);
                let engine =
                    SplatEngine::new(core.clone(), setup.shared, capacity, COLOUR_FORMAT).await?;
                Some(SplatScene {
                    engine,
                    layer: setup.layer,
                    given_up: false,
                })
            }
            None => None,
        };
        let initial_budget = splat
            .as_ref()
            .map_or(DEFAULT_SPLAT_CAPACITY, |scene| scene.engine.capacity());
        let count_readback = buffer(&core, 16, BUFFER_COPY_DST | BUFFER_MAP_READ);
        let timing = core.timestamps.then(|| Timing {
            set: core.device.create_query_set(
                &Descriptor::new()
                    .with("type", "timestamp")
                    .with("count", 8)
                    .build(),
            ),
            resolve: buffer(&core, 64, BUFFER_QUERY_RESOLVE | BUFFER_COPY_SRC),
            read: buffer(&core, 64, BUFFER_COPY_DST | BUFFER_MAP_READ),
        });
        let renderer = Self {
            core,
            pipelines,
            frame_buffer,
            frame_group,
            terrain_buffer,
            terrain_group,
            post_buffer,
            decal_buffer,
            figure_buffer,
            targets: None,
            cache: Rc::new(RefCell::new(HashMap::new())),
            backdrop_stem: scene.backdrop.stem.clone(),
            ground_stem: scene.ground.stem.clone(),
            dark_ground,
            clear_pixel,
            splat,
            measured: Rc::new(RefCell::new(Measured::default())),
            timing,
            count_readback,
            readback_busy: Rc::new(std::cell::Cell::new(false)),
            quality: Quality {
                splat_scale: 1.0,
                splat_budget: initial_budget,
                min_contribution: BASE_MIN_CONTRIBUTION,
                render_scale: 1.0,
                tilt_shift: true,
            },
            frame_ms_average: 16.0,
            slow_frames: 0,
            fast_frames: 0,
            floor_frames: 0,
            ladder_frozen: false,
            debug_switches: 0,
            spot: (0.0, 0.0, 0.0),
            last_clock_s: 0.0,
            frames_seen: 0,
            stats: RenderStats::default(),
        };
        renderer.request(&renderer.backdrop_stem.clone(), false, true);
        renderer.request(&renderer.ground_stem.clone(), false, true);
        Ok(renderer)
    }

    pub fn core(&self) -> &Rc<GpuCore> {
        &self.core
    }

    pub fn stats(&self) -> RenderStats {
        let mut stats = self.stats;
        let measured = self.measured.borrow();
        stats.sort_ms = measured.sort_ms;
        stats.splat_ms = measured.splat_ms;
        stats.overlay_ms = measured.overlay_ms;
        stats.post_ms = measured.post_ms;
        stats.gpu_ms = measured.gpu_ms;
        stats.splats_visible = measured.visible;
        stats.splat_scale = self.quality.splat_scale;
        stats
    }

    /// True when the scene has splats that failed to load or were given up for speed, so the
    /// painted ground is what is drawn.
    pub fn splat_unusable(&self) -> bool {
        self.splat.as_ref().is_some_and(|scene| {
            scene.given_up || scene.engine.shared().borrow().state == LoadState::Failed
        })
    }

    /// Switches the painted ground and backdrop to other stems (the fallback for a failed scan).
    pub fn set_painted(&mut self, ground_stem: &str, backdrop_stem: &str) {
        self.ground_stem = ground_stem.to_owned();
        self.backdrop_stem = backdrop_stem.to_owned();
        self.request(ground_stem, false, true);
        self.request(backdrop_stem, false, true);
    }

    /// The loader's side of the splat scene, when this renderer has one.
    pub fn splat_shared(&self) -> Option<SharedSplats> {
        self.splat
            .as_ref()
            .map(|scene| scene.engine.shared().clone())
    }

    /// The bind group layout a resident chunk is built against.
    pub fn splat_chunk_layout(&self) -> Option<JsValue> {
        self.splat
            .as_ref()
            .map(|scene| scene.engine.chunk_layout().clone())
    }

    fn request(&self, stem: &str, premultiplied: bool, repeat: bool) {
        if self.cache.borrow().contains_key(stem) {
            return;
        }
        self.cache
            .borrow_mut()
            .insert(stem.to_owned(), Slot::Loading);
        let core = self.core.clone();
        let cache = self.cache.clone();
        let stem = stem.to_owned();
        wasm_bindgen_futures::spawn_local(async move {
            let url = format!("{GENERATED}/{stem}");
            let slot = match textures::load_image(&core, &url, premultiplied, repeat).await {
                Ok(mut texture) => {
                    if premultiplied {
                        // Measure the visible figure so it is scaled and planted by its own box.
                        let pixels = textures::read_texture_rgba(
                            &core,
                            &texture.texture,
                            texture.width,
                            texture.height,
                        )
                        .await;
                        if let Ok(pixels) = pixels
                            && let Some(bounds) = crate::battle::visible_bounds(
                                &pixels,
                                texture.width,
                                texture.height,
                                24,
                                3,
                            )
                        {
                            texture.bounds = bounds;
                        }
                    }
                    Slot::Ready(Rc::new(texture))
                }
                Err(_) => Slot::Failed,
            };
            cache.borrow_mut().insert(stem, slot);
        });
    }

    fn ready(&self, stem: &str) -> Option<Rc<ImageTexture>> {
        match self.cache.borrow().get(stem) {
            Some(Slot::Ready(texture)) => Some(texture.clone()),
            _ => None,
        }
    }

    /// Pictures still being fetched or decoded.
    pub fn pending_loads(&self) -> usize {
        self.cache
            .borrow()
            .values()
            .filter(|slot| matches!(slot, Slot::Loading))
            .count()
    }

    /// Feeds the frame-time ladder. After a slow stretch it skips more small splats, then lowers
    /// the splat budget, then the render scale, then drops the tilt-shift blur; with sustained
    /// headroom it climbs back up in the reverse order, one step at a time. A scan that is still
    /// slow with everything at its floor is given up for the painted ground.
    /// Holds the quality at the given values and stops the ladder (benchmarks).
    pub fn freeze_quality(
        &mut self,
        budget: Option<u32>,
        min_contribution: Option<f32>,
        render_scale: Option<f32>,
        splat_scale: Option<f32>,
    ) {
        self.ladder_frozen = true;
        if let Some(value) = splat_scale {
            self.quality.splat_scale = value.clamp(0.25, 1.0);
        }
        if let Some(budget) = budget {
            self.set_budget(budget.min(self.quality_capacity()));
        }
        if let Some(value) = min_contribution {
            self.quality.min_contribution = value;
        }
        if let Some(value) = render_scale {
            self.quality.render_scale = value.clamp(0.25, 1.0);
        }
    }

    fn quality_capacity(&self) -> u32 {
        self.splat
            .as_ref()
            .map_or(DEFAULT_SPLAT_CAPACITY, |scene| scene.engine.capacity())
    }

    pub fn observe_frame_ms(&mut self, milliseconds: f32) {
        if self.ladder_frozen {
            return;
        }
        self.frame_ms_average += (milliseconds.clamp(0.0, 200.0) - self.frame_ms_average) * 0.1;
        self.frames_seen += 1;
        let has_splats = self.splat.as_ref().is_some_and(|scene| !scene.given_up);
        if self.frame_ms_average > SLOW_FRAME_MS {
            self.slow_frames += 1;
            self.fast_frames = 0;
        } else if self.frame_ms_average < FAST_FRAME_MS {
            self.fast_frames += 1;
            self.slow_frames = 0;
        } else {
            self.slow_frames = 0;
            self.fast_frames = 0;
        }
        if self.slow_frames >= STEP_DOWN_FRAMES {
            self.slow_frames = 0;
            if !self.step_down(has_splats) {
                self.floor_frames += STEP_DOWN_FRAMES;
            }
        }
        if self.fast_frames >= STEP_UP_FRAMES {
            self.fast_frames = 0;
            self.floor_frames = 0;
            self.step_up(has_splats);
        }
        if has_splats
            && self.floor_frames >= GIVE_UP_FRAMES
            && self.frame_ms_average > GIVE_UP_FRAME_MS
            && let Some(scene) = &mut self.splat
        {
            scene.given_up = true;
        }
    }

    fn set_budget(&mut self, budget: u32) {
        self.quality.splat_budget = budget;
        if let Some(scene) = &self.splat {
            let mut shared = scene.engine.shared().borrow_mut();
            shared.budget = budget;
            shared.budget_changed = true;
        }
    }

    /// One step toward cheaper frames; false when nothing is left to give. Splat fill rate is
    /// what costs time, so the scan's own resolution goes first, then how many splats are drawn.
    fn step_down(&mut self, has_splats: bool) -> bool {
        if has_splats && self.quality.splat_scale > MIN_SPLAT_SCALE + 0.01 {
            self.quality.splat_scale = (self.quality.splat_scale - 0.12).max(MIN_SPLAT_SCALE);
        } else if has_splats && self.quality.splat_budget > MIN_SPLAT_BUDGET {
            self.set_budget(
                ((self.quality.splat_budget as f32 * 0.75) as u32).max(MIN_SPLAT_BUDGET),
            );
        } else if has_splats && self.quality.min_contribution < MAX_MIN_CONTRIBUTION {
            self.quality.min_contribution =
                (self.quality.min_contribution * 1.7).min(MAX_MIN_CONTRIBUTION);
        } else if self.quality.render_scale > 0.55 {
            self.quality.render_scale -= 0.15;
        } else if self.quality.tilt_shift {
            self.quality.tilt_shift = false;
        } else {
            return false;
        }
        true
    }

    fn step_up(&mut self, has_splats: bool) {
        let capacity = self.quality_capacity();
        if !self.quality.tilt_shift {
            self.quality.tilt_shift = true;
        } else if self.quality.render_scale < 1.0 {
            self.quality.render_scale = (self.quality.render_scale + 0.15).min(1.0);
        } else if has_splats && self.quality.min_contribution > BASE_MIN_CONTRIBUTION {
            self.quality.min_contribution =
                (self.quality.min_contribution / 1.7).max(BASE_MIN_CONTRIBUTION);
        } else if has_splats && self.quality.splat_budget < capacity {
            self.set_budget(((self.quality.splat_budget as f32 * 1.25) as u32).min(capacity));
        } else if has_splats && self.quality.splat_scale < 1.0 {
            self.quality.splat_scale = (self.quality.splat_scale + 0.12).min(1.0);
        }
    }

    fn ensure_targets(&mut self, canvas: (u32, u32)) {
        let width = ((canvas.0 as f32 * self.quality.render_scale) as u32).max(320);
        let height = ((canvas.1 as f32 * self.quality.render_scale) as u32).max(180);
        let splat_scale = self.quality.splat_scale;
        if let Some(scene) = &mut self.splat {
            scene.engine.ensure_targets(
                ((width as f32 * splat_scale) as u32).max(160),
                ((height as f32 * splat_scale) as u32).max(90),
            );
        }
        if let Some(targets) = &self.targets
            && targets.width == width
            && targets.height == height
        {
            return;
        }
        if let Some(old) = self.targets.take() {
            old.scene.destroy_texture();
            old.depth.destroy_texture();
        }
        let scene = self
            .core
            .create_texture(width, height, COLOUR_FORMAT, COLOUR_TARGET_USAGE, 1);
        let depth = self
            .core
            .create_texture(width, height, DEPTH_FORMAT, COLOUR_TARGET_USAGE, 1);
        let post_group = bind_group(
            &self.core,
            &self.core.layout_post,
            vec![
                entry_buffer(0, &self.post_buffer),
                Descriptor::new()
                    .with("binding", 1)
                    .with("resource", scene.create_view())
                    .build(),
                Descriptor::new()
                    .with("binding", 2)
                    .with("resource", self.core.sampler_clamp.clone())
                    .build(),
            ],
        );
        self.targets = Some(Targets {
            width,
            height,
            scene,
            depth,
            post_group,
        });
    }

    fn write_frame(
        &self,
        spot: (f32, f32, f32),
        frame: &FrameView<'_>,
        scene: &BattleScene,
        size: (u32, u32),
        forward: Vec3,
    ) {
        let pose = frame.camera;
        let aspect = size.0 as f32 / size.1 as f32;
        let view = pose.view();
        let view_proj = pose.projection(aspect, NEAR, FAR).mul(&view);
        let eye = pose.eye();
        let right = forward.cross(Vec3::UP);
        let flat = Vec3::new(right.x, 0.0, right.z).normalized();
        let flat = if flat == Vec3::ZERO {
            Vec3::new(1.0, 0.0, 0.0)
        } else {
            flat
        };
        let focal = 0.5 * size.1 as f32 / (pose.fov_deg.to_radians() * 0.5).tan();
        let centre = scene.grid.centre();
        let mut data = Vec::with_capacity(60);
        data.extend_from_slice(&view_proj.0);
        data.extend_from_slice(&view.0);
        data.extend_from_slice(&[eye.x, eye.y, eye.z, frame.clock_s]);
        data.extend_from_slice(&[flat.x, flat.y, flat.z, aspect]);
        data.extend_from_slice(&[size.0 as f32, size.1 as f32, focal, 0.0]);
        // Tiles are lifted along the line of sight, which leaves their place on screen unchanged
        // and only keeps nearby grass and rubble from hiding them.
        let decal_lift = if scene.splat.is_some() { DECAL_LIFT_M } else { 0.0 };
        data.extend_from_slice(&[centre.x, centre.z, 8.5, decal_lift]);
        data.extend_from_slice(&[
            scene.ground.centre[0],
            scene.ground.centre[1],
            scene.ground.size_m * 0.5,
            scene.ground.falloff_m,
        ]);
        data.extend_from_slice(&[
            centre.z - scene.backdrop.distance_m,
            scene.backdrop.width_m * 0.5,
            scene.backdrop.base_m,
            scene.backdrop.height_m,
        ]);
        data.extend_from_slice(&[spot.0, spot.1, 3.4, spot.2]);
        self.core
            .queue
            .write_buffer(&self.frame_buffer, 0.0, &float_array(&data));
    }

    fn write_post(&self, frame: &FrameView<'_>, scene: &BattleScene, size: (u32, u32)) {
        let grade = scene.theme.grade(scene.grade_strength);
        let (flash_rgb, flash_strength) = match frame.flash {
            Some(flash) => (
                match flash.tone {
                    FlashTone::Impact => [1.0, 0.82, 0.7],
                    FlashTone::Critical => [1.0, 0.95, 0.75],
                    FlashTone::Heal => [0.55, 1.0, 0.7],
                },
                flash.strength,
            ),
            None => ([0.0; 3], 0.0),
        };
        let mut data = Vec::with_capacity(28);
        data.extend_from_slice(&[
            grade.exposure,
            grade.contrast,
            grade.saturation,
            if grade.enabled { 1.0 } else { 0.0 },
        ]);
        data.extend_from_slice(&[grade.shadow[0], grade.shadow[1], grade.shadow[2], 0.0]);
        data.extend_from_slice(&[
            grade.highlight[0],
            grade.highlight[1],
            grade.highlight[2],
            0.0,
        ]);
        data.extend_from_slice(&[0.52, 0.5, 0.4, 3.2]);
        data.extend_from_slice(&[
            0.62,
            flash_strength,
            frame.fade,
            if self.quality.tilt_shift { 1.0 } else { 0.0 },
        ]);
        data.extend_from_slice(&[flash_rgb[0], flash_rgb[1], flash_rgb[2], 0.0]);
        data.extend_from_slice(&[
            frame.shake_px.0,
            frame.shake_px.1,
            1.0 / size.0 as f32,
            1.0 / size.1 as f32,
        ]);
        self.core
            .queue
            .write_buffer(&self.post_buffer, 0.0, &float_array(&data));
    }

    fn decals(&self, frame: &FrameView<'_>, grid: &Grid) -> Vec<f32> {
        let mut out = Vec::new();
        let half = grid.cell_m() * 0.5;
        let centre = grid.centre();
        let mut push = |shape: [f32; 4], colour: [f32; 4], params: [f32; 3]| {
            if out.len() / DECAL_STRIDE < MAX_DECALS {
                out.extend_from_slice(&shape);
                out.extend_from_slice(&colour);
                out.extend_from_slice(&[params[0], params[1], params[2], 0.0]);
            }
        };
        let highlighted = |cell: crate::battle::Cell| {
            frame
                .highlight
                .filter(|highlight| highlight.cells.contains(&cell))
                .map(|highlight| highlight.kind)
        };
        for cell in grid.walkable_cells() {
            let at = grid.cell_centre(cell);
            let distance = ((at.x - centre.x).powi(2) + (at.z - centre.z).powi(2)).sqrt();
            let fade = 1.0 - 0.7 * ((distance - 3.0) / 6.0).clamp(0.0, 1.0);
            let shape = [at.x, at.z, half, half];
            match highlighted(cell) {
                Some(HighlightKind::Move) => {
                    push(shape, [0.45, 0.8, 1.0, 1.0], [1.0, 0.0, 0.0]);
                }
                Some(HighlightKind::Attack) => {
                    push(shape, [1.0, 0.3, 0.25, 1.0], [2.0, 0.0, 0.0]);
                }
                None => push(shape, [0.96, 0.78, 0.42, 0.62 * fade], [0.0, 0.0, 0.0]),
            }
        }
        for figure in &frame.figures {
            let size = 0.5 + figure.height_m * 0.12;
            let alpha = (0.62 * figure.alpha - 0.2 * figure.lift).max(0.0);
            push(
                [
                    figure.position.x,
                    figure.position.z + 0.05,
                    size,
                    size * 0.6,
                ],
                [0.0, 0.0, 0.0, alpha],
                [3.0, 0.0, 0.0],
            );
        }
        for figure in &frame.figures {
            let colour = match figure.side {
                Side::Hero => [0.5, 0.86, 1.0, 1.0],
                Side::Enemy => [1.0, 0.34, 0.28, 1.0],
            };
            let active = if figure.active { 1.0 } else { 0.0 };
            push(
                [figure.position.x, figure.position.z, half, half],
                colour,
                [4.0, 0.0, active],
            );
        }
        out
    }

    fn figures(&mut self, frame: &FrameView<'_>) -> (Vec<f32>, Vec<Rc<ImageTexture>>) {
        let mut data = Vec::new();
        let mut bound = Vec::new();
        for figure in frame.figures.iter().take(MAX_FIGURES) {
            let cutout = figure.figure.as_deref().and_then(|stem| {
                self.request(stem, true, false);
                self.ready(stem)
            });
            let portrait = if cutout.is_none() {
                figure.portrait.as_deref().and_then(|stem| {
                    self.request(stem, true, false);
                    self.ready(stem)
                })
            } else {
                None
            };
            let (texture, kind, has_texture) = match (cutout, portrait) {
                (Some(texture), _) => (texture, 0.0, 1.0),
                (None, Some(texture)) => (texture, 1.0, 1.0),
                (None, None) => (self.clear_pixel.clone(), 1.0, 0.0),
            };
            let aspect = texture.width as f32 / texture.height as f32;
            data.extend_from_slice(&[
                figure.position.x,
                figure.position.y,
                figure.position.z,
                figure.height_m,
            ]);
            data.extend_from_slice(&[
                figure.lean,
                figure.alpha,
                figure.red_flash,
                figure.heal_glow,
            ]);
            data.extend_from_slice(&[
                figure.tint[0],
                figure.tint[1],
                figure.tint[2],
                figure.breath,
            ]);
            data.extend_from_slice(&[
                aspect,
                kind,
                has_texture,
                if figure.side == Side::Enemy { 1.0 } else { 0.0 },
            ]);
            data.extend_from_slice(&[
                texture.bounds.top,
                texture.bounds.bottom,
                texture.bounds.centre(),
                0.0,
            ]);
            bound.push(texture);
        }
        (data, bound)
    }

    /// Draws one frame and presents it. `canvas` is the drawing-buffer size in device pixels.
    pub fn render(&mut self, frame: &FrameView<'_>, scene: &BattleScene, canvas: (u32, u32)) {
        self.ensure_targets(canvas);
        let Some(size) = self.targets.as_ref().map(|t| (t.width, t.height)) else {
            return;
        };
        let pose = frame.camera;
        let forward = (pose.target - pose.eye()).normalized();
        let dt = (frame.clock_s - self.last_clock_s).clamp(0.0, 0.1);
        self.last_clock_s = frame.clock_s;
        let acting = frame.figures.iter().find(|figure| figure.active);
        let ease = 1.0 - (-6.0 * dt).exp();
        match acting {
            Some(figure) => {
                if self.spot.2 < 0.02 {
                    self.spot.0 = figure.position.x;
                    self.spot.1 = figure.position.z;
                }
                self.spot.0 += (figure.position.x - self.spot.0) * ease;
                self.spot.1 += (figure.position.z - self.spot.1) * ease;
                self.spot.2 += (1.0 - self.spot.2) * ease;
            }
            None => self.spot.2 += (0.0 - self.spot.2) * ease,
        }
        self.write_frame(self.spot, frame, scene, size, forward);
        self.write_post(frame, scene, size);
        let decals = self.decals(frame, &scene.grid);
        let decal_count = decals.len() / DECAL_STRIDE;
        if decal_count > 0 {
            self.core
                .queue
                .write_buffer(&self.decal_buffer, 0.0, &float_array(&decals));
        }
        let (figure_data, figure_textures) = self.figures(frame);
        if !figure_data.is_empty() {
            self.core
                .queue
                .write_buffer(&self.figure_buffer, 0.0, &float_array(&figure_data));
        }
        let backdrop = self
            .ready(&self.backdrop_stem)
            .map(|texture| texture.bind_group.clone());
        let ground = self.ready(&self.ground_stem);

        let (scene_view, depth_view, post_group) = match &self.targets {
            Some(targets) => (
                targets.scene.create_view(),
                targets.depth.create_view(),
                targets.post_group.clone(),
            ),
            None => return,
        };
        let compute_encoder = self.core.device.create_command_encoder();
        let encoder = self.core.device.create_command_encoder();
        let measuring = self.timing.is_some()
            && self.splat.is_some()
            && self.frames_seen.is_multiple_of(20)
            && !self.readback_busy.get();
        let drawing = self.prepare_splats(&compute_encoder, frame, scene, size, measuring);
        let measuring = measuring && drawing;
        let pass = if drawing {
            self.splat_passes(&encoder, &scene_view, &depth_view, measuring)
        } else {
            let pass = encoder.begin_render_pass(
                &Descriptor::new()
                    .with(
                        "colorAttachments",
                        array([Descriptor::new()
                            .with("view", scene_view)
                            .with(
                                "clearValue",
                                Descriptor::new()
                                    .with("r", 0.015)
                                    .with("g", 0.02)
                                    .with("b", 0.03)
                                    .with("a", 1.0)
                                    .build(),
                            )
                            .with("loadOp", "clear")
                            .with("storeOp", "store")
                            .build()]),
                    )
                    .with(
                        "depthStencilAttachment",
                        Descriptor::new()
                            .with("view", depth_view)
                            .with("depthClearValue", 1.0)
                            .with("depthLoadOp", "clear")
                            .with("depthStoreOp", "store")
                            .build(),
                    )
                    .build(),
            );
            if let Some(group) = &backdrop {
                pass.set_pipeline(&self.pipelines.backdrop);
                pass.set_bind_group(0, &self.frame_group);
                pass.set_bind_group(1, group);
                pass.draw(6, 1, 0, 0);
            }
            let group = ground
                .as_ref()
                .map_or(&self.dark_ground.bind_group, |texture| &texture.bind_group);
            pass.set_pipeline(&self.pipelines.ground);
            pass.set_bind_group(0, &self.frame_group);
            pass.set_bind_group(1, group);
            pass.draw(6, 1, 0, 0);
            pass
        };
        if decal_count > 0 {
            pass.set_pipeline(&self.pipelines.decal);
            pass.set_bind_group(0, &self.frame_group);
            pass.set_bind_group(1, &self.terrain_group);
            pass.set_vertex_buffer(0, &self.decal_buffer, 0.0, (decals.len() * 4) as f64);
            pass.draw(DECAL_VERTICES, decal_count as u32, 0, 0);
        }
        pass.set_pipeline(&self.pipelines.figure);
        pass.set_bind_group(0, &self.frame_group);
        for (index, texture) in figure_textures.iter().enumerate() {
            pass.set_bind_group(1, &texture.bind_group);
            let stride = (FIGURE_STRIDE * 4) as f64;
            pass.set_vertex_buffer(0, &self.figure_buffer, index as f64 * stride, stride);
            pass.draw(6, 1, 0, 0);
        }
        pass.end();

        let output = self.core.context.current_texture();
        let mut post_descriptor = Descriptor::new();
        if measuring {
            post_descriptor = post_descriptor.with("timestampWrites", self.stamps(6));
        }
        let post = encoder.begin_render_pass(
            &post_descriptor
                .with(
                    "colorAttachments",
                    array([Descriptor::new()
                        .with("view", output.create_view())
                        .with(
                            "clearValue",
                            Descriptor::new()
                                .with("r", 0.0)
                                .with("g", 0.0)
                                .with("b", 0.0)
                                .with("a", 1.0)
                                .build(),
                        )
                        .with("loadOp", "clear")
                        .with("storeOp", "store")
                        .build()]),
                )
                .build(),
        );
        if self.debug_switches & 4 == 0 {
            post.set_pipeline(&self.pipelines.post);
            post.set_bind_group(0, &post_group);
            post.draw(3, 1, 0, 0);
        }
        post.end();
        self.submit_frame(compute_encoder, encoder, measuring);
        self.stats.scale = self.quality.render_scale;
        self.stats.budget = self.quality.splat_budget;
        self.stats.min_contribution = self.quality.min_contribution;
        self.stats.given_up = self.splat.as_ref().is_some_and(|scene| scene.given_up);
        self.stats.splat_state = self
            .splat
            .as_ref()
            .map(|scene| scene.engine.shared().borrow().state);
        self.stats.splats_planned = self.splat.as_ref().map_or(0, |scene| scene.engine.active());
    }

    /// Encodes the decode, project and sort work for this frame. True when a splat scene has
    /// something to draw (and has not been given up for the painted ground).
    fn prepare_splats(
        &mut self,
        encoder: &super::sys::GpuCommandEncoder,
        frame: &FrameView<'_>,
        scene: &BattleScene,
        size: (u32, u32),
        measuring: bool,
    ) -> bool {
        let quality = self.quality;
        let stamps = measuring.then(|| self.stamps(0));
        let depth_off = self.debug_switches & 1 != 0;
        let Some(splats) = &mut self.splat else {
            return false;
        };
        if splats.given_up {
            return false;
        }
        let pose = frame.camera;
        let aspect = size.0 as f32 / size.1 as f32;
        let view = pose.view();
        let projection = pose.projection(aspect, NEAR, FAR);
        let view_proj = projection.mul(&view);
        let eye = pose.eye();
        let layer = &splats.layer;
        let grade = layer.grade;
        let centre = scene.grid.centre();
        let lo = (
            ((size.0 as f32 * quality.splat_scale) as u32).max(160),
            ((size.1 as f32 * quality.splat_scale) as u32).max(90),
        );
        let focal_px = 0.5 * lo.1 as f32 / (pose.fov_deg.to_radians() * 0.5).tan();
        let params = FrameParams {
            view: view.0,
            view_proj: view_proj.0,
            eye: [eye.x, eye.y, eye.z],
            sort_range_m: 900.0,
            viewport: (lo.0 as f32, lo.1 as f32),
            focal_px,
            near_cull_m: 0.2,
            min_extent_px: 1.0,
            min_contribution: quality.min_contribution,
            exposure_gain: grade.exposure_ev.exp2(),
            saturation: grade.saturation,
            ambient: grade.ambient,
            pool_m: grade.pool_m,
            tint: grade.tint,
            pool_centre: (centre.x, centre.z),
            projection_m10: projection.0[10],
            projection_m14: projection.0[14],
            depth_bias_m: layer.depth_bias_m,
            sky: grade.sky,
            clip: layer.clip.map_or([0.0; 4], |(height, radius)| {
                [height, radius * radius, centre.x, centre.z]
            }),
            full_size: (size.0 as f32, size.1 as f32),
        };
        let turn = crate::battle::splat::SplatTransform {
            scale: layer.scale,
            offset_y: layer.offset_y,
            rot_y_deg: layer.rot_y_deg,
            translate: layer.translate,
            orient: layer.orient,
        }
        .rotation();
        let shift = layer.translate + Vec3::new(0.0, layer.offset_y, 0.0);
        let placement = [
            turn.x,
            turn.y,
            turn.z,
            turn.w,
            shift.x,
            shift.y,
            shift.z,
            layer.scale,
        ];
        splats.engine.depth_threshold = if depth_off { 2.0 } else { 0.4 };
        splats.engine.prepare(encoder, &params, placement, stamps)
    }

    /// A `timestampWrites` descriptor writing stamps `first` and `first + 1`.
    fn stamps(&self, first: u32) -> JsValue {
        let set = self.timing.as_ref().map(|timing| timing.set.clone());
        Descriptor::new()
            .with("querySet", set.map_or(JsValue::NULL, JsValue::from))
            .with("beginningOfPassWriteIndex", first)
            .with("endOfPassWriteIndex", first + 1)
            .build()
    }

    /// The two passes of a splat frame: the scan into colour plus accumulated depth, then a pass
    /// that resolves that depth into the depth buffer and stays open for tiles and figures.
    fn splat_passes(
        &self,
        encoder: &super::sys::GpuCommandEncoder,
        scene_view: &super::sys::GpuTextureView,
        depth_view: &super::sys::GpuTextureView,
        measuring: bool,
    ) -> GpuRenderPass {
        let Some(splats) = &self.splat else {
            return encoder.begin_render_pass(&Descriptor::new().build());
        };
        let views = splats.engine.target_views();
        let clear = |r: f32, g: f32, b: f32, a: f32| {
            Descriptor::new()
                .with("r", r)
                .with("g", g)
                .with("b", b)
                .with("a", a)
                .build()
        };
        let attachments: Vec<JsValue> = match views {
            Some((colour_view, accum_view)) => [colour_view, accum_view]
                .into_iter()
                .map(|view| {
                    Descriptor::new()
                        .with("view", view)
                        .with("clearValue", clear(0.0, 0.0, 0.0, 0.0))
                        .with("loadOp", "clear")
                        .with("storeOp", "store")
                        .build()
                })
                .collect(),
            None => Vec::new(),
        };
        let mut scan_descriptor = Descriptor::new().with("colorAttachments", array(attachments));
        if measuring {
            scan_descriptor = scan_descriptor.with("timestampWrites", self.stamps(2));
        }
        let scan = encoder.begin_render_pass(&scan_descriptor.build());
        if self.debug_switches & 2 == 0 {
            splats.engine.draw(&scan);
        }
        scan.end();
        let mut overlay_descriptor = Descriptor::new();
        if measuring {
            overlay_descriptor = overlay_descriptor.with("timestampWrites", self.stamps(4));
        }
        let pass = encoder.begin_render_pass(
            &overlay_descriptor
                .with(
                    "colorAttachments",
                    array([Descriptor::new()
                        .with("view", scene_view.clone())
                        .with("clearValue", clear(0.0, 0.0, 0.0, 1.0))
                        .with("loadOp", "clear")
                        .with("storeOp", "store")
                        .build()]),
                )
                .with(
                    "depthStencilAttachment",
                    Descriptor::new()
                        .with("view", depth_view.clone())
                        .with("depthClearValue", 1.0)
                        .with("depthLoadOp", "clear")
                        .with("depthStoreOp", "store")
                        .build(),
                )
                .build(),
        );
        splats.engine.resolve(&pass);
        pass
    }

    /// Submits the frame. On a measured frame the pass timestamps are resolved and the number of
    /// splats that survived culling is copied out; both are read back without blocking.
    fn submit_frame(
        &mut self,
        compute: super::sys::GpuCommandEncoder,
        render: super::sys::GpuCommandEncoder,
        measuring: bool,
    ) {
        let (Some(timing), true) = (&self.timing, measuring) else {
            self.core
                .queue
                .submit(&array([compute.finish().into(), render.finish().into()]));
            return;
        };
        if let Some(scene) = &self.splat {
            scene
                .engine
                .copy_visible_count(&compute, &self.count_readback);
        }
        render.resolve_query_set(&timing.set, 0, 8, &timing.resolve, 0.0);
        render.copy_buffer_to_buffer(&timing.resolve, 0.0, &timing.read, 0.0, 64.0);
        self.core
            .queue
            .submit(&array([compute.finish().into(), render.finish().into()]));
        let measured = self.measured.clone();
        let busy = self.readback_busy.clone();
        busy.set(true);
        let read = timing.read.clone();
        let count = self.count_readback.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let stamps =
                wasm_bindgen_futures::JsFuture::from(read.map_async(BUFFER_MAP_READ)).await;
            if stamps.is_ok() {
                let values = js_sys::BigUint64Array::new(&read.get_mapped_range());
                let at = |index: u32| values.get_index(index) as f64;
                let ms = |first: u32| ((at(first + 1) - at(first)) / 1.0e6).max(0.0) as f32;
                let begin = (0..8).step_by(2).map(&at).fold(f64::MAX, f64::min);
                let end = (1..8).step_by(2).map(&at).fold(0.0, f64::max);
                let mut value = measured.borrow_mut();
                let blend = |old: &mut f32, new: f32| *old += (new - *old) * 0.3;
                blend(&mut value.sort_ms, ms(0));
                blend(&mut value.splat_ms, ms(2));
                blend(&mut value.overlay_ms, ms(4));
                blend(&mut value.post_ms, ms(6));
                blend(&mut value.gpu_ms, ((end - begin) / 1.0e6).max(0.0) as f32);
            }
            read.unmap();
            let counted =
                wasm_bindgen_futures::JsFuture::from(count.map_async(BUFFER_MAP_READ)).await;
            if counted.is_ok() {
                let words = js_sys::Uint32Array::new(&count.get_mapped_range());
                measured.borrow_mut().visible = words.get_index(0);
            }
            count.unmap();
            busy.set(false);
        });
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        if let Some(targets) = self.targets.take() {
            targets.scene.destroy_texture();
            targets.depth.destroy_texture();
        }
        self.terrain_buffer.destroy_buffer();
        self.count_readback.destroy_buffer();
        self.splat.take();
    }
}
