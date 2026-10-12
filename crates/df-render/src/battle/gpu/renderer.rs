//! The WebGPU renderer: one scene pass (backdrop, ground or splats, grid decals, figures) into an
//! off-screen colour target, then a post pass to the canvas (grade, tilt-shift, vignette, flash).
//! It draws what [`Stage::frame`](crate::battle::Stage::frame) describes and owns no game state.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use wasm_bindgen::JsValue;

use super::{
    core::{COLOUR_TARGET_USAGE, GpuCore, UnavailableReason},
    shaders,
    sys::{
        BUFFER_COPY_DST, BUFFER_STORAGE, BUFFER_UNIFORM, BUFFER_VERTEX, Descriptor, GpuBindGroup,
        GpuBuffer, GpuRenderPass, GpuRenderPipeline, GpuTexture, array, float_array,
    },
    textures::{self, ImageTexture},
};
use crate::battle::{
    director::FlashTone,
    grid::Grid,
    math::Vec3,
    model::{HighlightKind, Side},
    scene_file::BattleScene,
    splat::{SplatCloud, SplatSorter},
    stage::FrameView,
};

const GENERATED: &str = "/assets/ui/generated";
const DEPTH_FORMAT: &str = "depth32float";
const COLOUR_FORMAT: &str = "rgba8unorm";
const DECAL_STRIDE: usize = 12;
const FIGURE_STRIDE: usize = 20;
const MAX_DECALS: usize = 768;
const MAX_FIGURES: usize = 16;
const NEAR: f32 = 0.1;
const FAR: f32 = 150.0;
pub const DEFAULT_SPLAT_CAP: usize = 600_000;

enum Slot {
    Loading,
    Ready(Rc<ImageTexture>),
    Failed,
}

type Cache = Rc<RefCell<HashMap<String, Slot>>>;

/// Splats waiting to be uploaded, produced by the loader task.
pub type PendingSplats = Rc<RefCell<Option<SplatCloud>>>;

struct Pipelines {
    backdrop: GpuRenderPipeline,
    ground: GpuRenderPipeline,
    decal: GpuRenderPipeline,
    figure: GpuRenderPipeline,
    splat: GpuRenderPipeline,
    post: GpuRenderPipeline,
}

struct Targets {
    width: u32,
    height: u32,
    scene: GpuTexture,
    depth: GpuTexture,
    post_group: GpuBindGroup,
}

struct SplatGpu {
    cloud: SplatCloud,
    order_buffer: GpuBuffer,
    splat_buffer: GpuBuffer,
    group: GpuBindGroup,
    drawn: u32,
    sorted_cap: usize,
}

/// Cost knobs the frame-time budget turns down on weak devices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quality {
    pub splat_cap: usize,
    pub render_scale: f32,
    pub tilt_shift: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    pub splats_loaded: usize,
    pub splats_drawn: u32,
    pub scale: f32,
    pub splat_cap: usize,
}

pub struct Renderer {
    core: Rc<GpuCore>,
    pipelines: Pipelines,
    frame_buffer: GpuBuffer,
    frame_group: GpuBindGroup,
    post_buffer: GpuBuffer,
    decal_buffer: GpuBuffer,
    figure_buffer: GpuBuffer,
    targets: Option<Targets>,
    cache: Cache,
    backdrop_stem: String,
    ground_stem: String,
    dark_ground: ImageTexture,
    clear_pixel: Rc<ImageTexture>,
    splat: Option<SplatGpu>,
    pending_splat: PendingSplats,
    sorter: SplatSorter,
    pub quality: Quality,
    frame_ms_average: f32,
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
        pending_splat: PendingSplats,
    ) -> Result<Self, (UnavailableReason, String)> {
        core.device.push_error_scope("validation");
        let frame_layout = core.pipeline_layout(&[&core.layout_frame]);
        let texture_layout = core.pipeline_layout(&[&core.layout_frame, &core.layout_texture]);
        let splat_layout = core.pipeline_layout(&[&core.layout_frame, &core.layout_splat]);
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
                    layout: &frame_layout,
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
            splat: pipeline(
                &core,
                PipelineSpec {
                    wgsl: &shaders::splat(),
                    layout: &splat_layout,
                    buffers: vec![],
                    topology: "triangle-strip",
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
        let renderer = Self {
            core,
            pipelines,
            frame_buffer,
            frame_group,
            post_buffer,
            decal_buffer,
            figure_buffer,
            targets: None,
            cache: Rc::new(RefCell::new(HashMap::new())),
            backdrop_stem: scene.backdrop.stem.clone(),
            ground_stem: scene.ground.stem.clone(),
            dark_ground,
            clear_pixel,
            splat: None,
            pending_splat,
            sorter: SplatSorter::default(),
            quality: Quality {
                splat_cap: DEFAULT_SPLAT_CAP,
                render_scale: 1.0,
                tilt_shift: true,
            },
            frame_ms_average: 16.0,
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
        self.stats
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

    /// Feeds the frame-time budget. After a sustained slow stretch it lowers the splat cap, then
    /// the render scale, then drops the tilt-shift blur.
    pub fn observe_frame_ms(&mut self, milliseconds: f32) {
        self.frame_ms_average += (milliseconds.clamp(0.0, 200.0) - self.frame_ms_average) * 0.08;
        self.frames_seen += 1;
        if !self.frames_seen.is_multiple_of(40) || self.frame_ms_average < 26.0 {
            return;
        }
        if self.splat.is_some() && self.quality.splat_cap > 80_000 {
            self.quality.splat_cap = (self.quality.splat_cap as f32 * 0.65) as usize;
            self.sorter.invalidate();
        } else if self.quality.render_scale > 0.55 {
            self.quality.render_scale -= 0.25;
        } else {
            self.quality.tilt_shift = false;
        }
    }

    fn ensure_targets(&mut self, canvas: (u32, u32)) {
        let width = ((canvas.0 as f32 * self.quality.render_scale) as u32).max(320);
        let height = ((canvas.1 as f32 * self.quality.render_scale) as u32).max(180);
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

    fn upload_pending_splats(&mut self) {
        let Some(cloud) = self.pending_splat.borrow_mut().take() else {
            return;
        };
        if cloud.is_empty() {
            return;
        }
        let bytes = cloud.len() * 48;
        let splat_buffer = buffer(&self.core, bytes, BUFFER_STORAGE | BUFFER_COPY_DST);
        let order_buffer = buffer(
            &self.core,
            cloud.len() * 4,
            BUFFER_STORAGE | BUFFER_COPY_DST,
        );
        self.core
            .queue
            .write_buffer(&splat_buffer, 0.0, &float_array(&cloud.gpu_floats()));
        let group = bind_group(
            &self.core,
            &self.core.layout_splat,
            vec![
                entry_buffer(0, &splat_buffer),
                entry_buffer(1, &order_buffer),
            ],
        );
        self.stats.splats_loaded = cloud.len();
        self.sorter.invalidate();
        self.splat = Some(SplatGpu {
            cloud,
            order_buffer,
            splat_buffer,
            group,
            drawn: 0,
            sorted_cap: 0,
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
        data.extend_from_slice(&[centre.x, centre.z, 8.5, 0.0]);
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

    fn draw_splats(&mut self, pass: &GpuRenderPass, frame: &FrameView<'_>, forward: Vec3) {
        let cap = self.quality.splat_cap;
        let eye = frame.camera.eye();
        let Some(splat) = &mut self.splat else {
            return;
        };
        if self.sorter.needs_sort(eye, forward) || splat.sorted_cap != cap {
            let active = cap.min(splat.cloud.len());
            let order = self.sorter.sort(splat.cloud.splats(), active, eye, forward);
            splat.drawn = order.len() as u32;
            splat.sorted_cap = cap;
            if !order.is_empty() {
                self.core.queue.write_buffer(
                    &splat.order_buffer,
                    0.0,
                    &js_sys::Uint32Array::from(order),
                );
            }
        }
        self.stats.splats_drawn = splat.drawn;
        if splat.drawn == 0 {
            return;
        }
        pass.set_pipeline(&self.pipelines.splat);
        pass.set_bind_group(0, &self.frame_group);
        pass.set_bind_group(1, &splat.group);
        pass.draw(4, splat.drawn, 0, 0);
    }

    /// Draws one frame and presents it. `canvas` is the drawing-buffer size in device pixels.
    pub fn render(&mut self, frame: &FrameView<'_>, scene: &BattleScene, canvas: (u32, u32)) {
        self.upload_pending_splats();
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
        let using_splats = self.splat.is_some();

        let (scene_view, depth_view, post_group) = match &self.targets {
            Some(targets) => (
                targets.scene.create_view(),
                targets.depth.create_view(),
                targets.post_group.clone(),
            ),
            None => return,
        };
        let encoder = self.core.device.create_command_encoder();
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
        if using_splats {
            self.draw_splats(&pass, frame, forward);
        } else {
            let group = ground
                .as_ref()
                .map_or(&self.dark_ground.bind_group, |texture| &texture.bind_group);
            pass.set_pipeline(&self.pipelines.ground);
            pass.set_bind_group(0, &self.frame_group);
            pass.set_bind_group(1, group);
            pass.draw(6, 1, 0, 0);
        }
        if decal_count > 0 {
            pass.set_pipeline(&self.pipelines.decal);
            pass.set_bind_group(0, &self.frame_group);
            pass.set_vertex_buffer(0, &self.decal_buffer, 0.0, (decals.len() * 4) as f64);
            pass.draw(6, decal_count as u32, 0, 0);
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
        let post = encoder.begin_render_pass(
            &Descriptor::new()
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
        post.set_pipeline(&self.pipelines.post);
        post.set_bind_group(0, &post_group);
        post.draw(3, 1, 0, 0);
        post.end();
        self.core.queue.submit(&array([encoder.finish().into()]));
        self.stats.scale = self.quality.render_scale;
        self.stats.splat_cap = self.quality.splat_cap;
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        if let Some(targets) = self.targets.take() {
            targets.scene.destroy_texture();
            targets.depth.destroy_texture();
        }
        if let Some(splat) = self.splat.take() {
            splat.order_buffer.destroy_buffer();
            splat.splat_buffer.destroy_buffer();
        }
    }
}
