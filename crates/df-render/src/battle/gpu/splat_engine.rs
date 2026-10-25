//! The GPU side of a splat scene: decoded splat records, the per-frame project / sort / draw
//! passes and the depth resolve.
//!
//! Records live in one storage buffer. A streamed scan fills it by decoding resident SOG chunk
//! textures with a compute shader (no CPU decode, no read-back); a small scene (`.splat`, a
//! single SOG bundle, the procedural ground) is uploaded as already-packed records. Each frame a
//! compute pass projects and culls every record, a counting sort over 65536 camera-relative
//! depth bins orders the survivors back to front, and one indirect draw paints them into the scene
//! colour and into a second target that accumulates coverage-weighted depth. A full-screen pass
//! then writes that depth into the real depth buffer so figures, tiles and props test against the
//! scan.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use wasm_bindgen::JsValue;

use super::{
    core::{GpuCore, UnavailableReason},
    splat_shaders,
    sys::{
        BUFFER_COPY_DST, BUFFER_COPY_SRC, BUFFER_INDIRECT, BUFFER_STORAGE, BUFFER_UNIFORM,
        Descriptor, GpuBindGroup, GpuBuffer, GpuCommandEncoder, GpuComputePipeline, GpuRenderPass,
        GpuRenderPipeline, GpuTexture, SHADER_COMPUTE, SHADER_FRAGMENT, SHADER_VERTEX, array,
        float_array,
    },
};
use crate::battle::{
    lod::{DecodePlan, MAX_RANGES},
    splat::RECORD_WORDS,
};

const DEPTH_BIN_COUNT: usize = 1 << 16;
const WORKGROUP: u32 = 256;
const UNIFORM_BYTES: usize = 320 + 512 + 32;
const CHUNK_PARAM_BYTES: usize = 64;
pub const ACCUM_FORMAT: &str = "rg16float";

/// A decoded-planes SOG chunk resident on the GPU.
pub struct ChunkGpu {
    pub group: GpuBindGroup,
    pub params: GpuBuffer,
    pub textures: Vec<GpuTexture>,
    pub codebooks: GpuBuffer,
}

impl Drop for ChunkGpu {
    fn drop(&mut self) {
        for texture in &self.textures {
            texture.destroy_texture();
        }
        self.params.destroy_buffer();
        self.codebooks.destroy_buffer();
    }
}

/// Scene-level facts the streamer reports and the host reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StreamStats {
    pub chunks_loaded: u32,
    pub chunks_wanted: u32,
    pub splats_planned: u32,
    pub first_draw_ms: Option<f64>,
    pub full_detail_ms: Option<f64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadState {
    Idle,
    Loading,
    /// The first coarse level is drawing; finer levels may still arrive.
    Showing,
    /// Everything wanted is resident.
    Complete,
    Failed,
}

/// State shared by the streaming task (which fills it) and the renderer (which consumes it).
pub struct Shared {
    pub chunks: HashMap<u32, ChunkGpu>,
    pub plan: Option<DecodePlan>,
    /// Already-packed records for a small scene, uploaded once.
    pub records: Option<Vec<u32>>,
    pub state: LoadState,
    pub stats: StreamStats,
    /// Set by the renderer when the frame ladder wants fewer or more splats.
    pub budget: u32,
    pub budget_changed: bool,
}

impl Shared {
    pub fn new(budget: u32) -> Self {
        Self {
            chunks: HashMap::new(),
            plan: None,
            records: None,
            state: LoadState::Idle,
            stats: StreamStats::default(),
            budget,
            budget_changed: false,
        }
    }
}

pub type SharedSplats = Rc<RefCell<Shared>>;

/// Everything the passes need from the camera and scene for one frame.
pub struct FrameParams {
    pub view: [f32; 16],
    pub view_proj: [f32; 16],
    pub eye: [f32; 3],
    pub sort_range_m: f32,
    pub viewport: (f32, f32),
    pub focal_px: f32,
    pub near_cull_m: f32,
    pub min_extent_px: f32,
    pub min_contribution: f32,
    pub exposure_gain: f32,
    pub saturation: f32,
    pub ambient: f32,
    pub pool_m: f32,
    pub tint: [f32; 3],
    pub pool_centre: (f32, f32),
    pub projection_m10: f32,
    pub projection_m14: f32,
    pub depth_bias_m: f32,
    pub sky: [f32; 3],
    /// The distance from the pool centre at which the scan has faded out (0: drawn everywhere).
    pub rim_m: f32,
    /// Ceiling cut: `[height, radius squared, centre x, centre z]`, radius 0 for none.
    pub clip: [f32; 4],
    /// The full-resolution scene target the scan is composited into.
    pub full_size: (f32, f32),
    /// Where each figure stands on the screen: `[left, top, right, bottom]` in normalised device
    /// coordinates, then `[view depth in metres, 1, 0, 0]`. Scan splats beside or behind a figure
    /// may not hide it (see `figure_clear`).
    pub figures: Vec<[f32; 8]>,
    pub fire_light: crate::battle::FireLight,
}

struct Layouts {
    sort: JsValue,
    draw: JsValue,
    resolve: JsValue,
    decode_chunk: JsValue,
    decode_global: JsValue,
}

struct Pipelines {
    project: GpuComputePipeline,
    scan: GpuComputePipeline,
    scatter: GpuComputePipeline,
    decode: GpuComputePipeline,
    draw: GpuRenderPipeline,
    resolve: GpuRenderPipeline,
}

/// The scan's own render targets, at the splat resolution (which may be below the scene's).
struct SplatTargets {
    accum: GpuTexture,
    colour: GpuTexture,
    group: GpuBindGroup,
    width: u32,
    height: u32,
}

pub struct SplatEngine {
    core: Rc<GpuCore>,
    shared: SharedSplats,
    capacity: u32,
    layouts: Layouts,
    pipelines: Pipelines,
    uniform: GpuBuffer,
    records: GpuBuffer,
    cache: GpuBuffer,
    keys: GpuBuffer,
    order: GpuBuffer,
    hist: GpuBuffer,
    counter: GpuBuffer,
    args: GpuBuffer,
    ranges: GpuBuffer,
    placement: GpuBuffer,
    sort_group: GpuBindGroup,
    draw_group: GpuBindGroup,
    decode_group: GpuBindGroup,
    targets: Option<SplatTargets>,
    active_splats: u32,
    /// Accumulated coverage a pixel needs before the scan counts as a surface for depth. Above 1
    /// the scan writes no depth at all (used to show what occlusion hides).
    pub depth_threshold: f32,
}

fn entry(binding: u32, visibility: u32, kind: &str, config: JsValue) -> JsValue {
    Descriptor::new()
        .with("binding", binding)
        .with("visibility", visibility)
        .with(kind, config)
        .build()
}

fn storage(read_only: bool) -> JsValue {
    Descriptor::new()
        .with(
            "type",
            if read_only {
                "read-only-storage"
            } else {
                "storage"
            },
        )
        .build()
}

fn uniform_entry(binding: u32, visibility: u32) -> JsValue {
    entry(
        binding,
        visibility,
        "buffer",
        Descriptor::new().with("type", "uniform").build(),
    )
}

fn storage_entry(binding: u32, visibility: u32, read_only: bool) -> JsValue {
    entry(binding, visibility, "buffer", storage(read_only))
}

fn data_texture(binding: u32, visibility: u32) -> JsValue {
    entry(
        binding,
        visibility,
        "texture",
        Descriptor::new()
            .with("sampleType", "unfilterable-float")
            .build(),
    )
}

fn layout(core: &GpuCore, entries: Vec<JsValue>) -> JsValue {
    core.device
        .create_bind_group_layout(&Descriptor::new().with("entries", array(entries)).build())
}

fn buffer(core: &GpuCore, size: usize, usage: u32) -> GpuBuffer {
    core.device.create_buffer(
        &Descriptor::new()
            .with("size", size as f64)
            .with("usage", usage)
            .build(),
    )
}

fn buffer_entry(binding: u32, buffer: &GpuBuffer) -> JsValue {
    Descriptor::new()
        .with("binding", binding)
        .with(
            "resource",
            Descriptor::new().with("buffer", buffer.clone()).build(),
        )
        .build()
}

fn group(core: &GpuCore, layout: &JsValue, entries: Vec<JsValue>) -> GpuBindGroup {
    core.device.create_bind_group(
        &Descriptor::new()
            .with("layout", layout.clone())
            .with("entries", array(entries))
            .build(),
    )
}

fn compute_pipeline(
    core: &GpuCore,
    wgsl: &str,
    entry_point: &str,
    layouts: &[&JsValue],
) -> GpuComputePipeline {
    let module = core
        .device
        .create_shader_module(&Descriptor::new().with("code", wgsl).build());
    core.device.create_compute_pipeline(
        &Descriptor::new()
            .with("layout", core.pipeline_layout(layouts))
            .with(
                "compute",
                Descriptor::new()
                    .with("module", module)
                    .with("entryPoint", entry_point)
                    .build(),
            )
            .build(),
    )
}

fn blend() -> JsValue {
    let component = || {
        Descriptor::new()
            .with("srcFactor", "one")
            .with("dstFactor", "one-minus-src-alpha")
            .with("operation", "add")
            .build()
    };
    Descriptor::new()
        .with("color", component())
        .with("alpha", component())
        .build()
}

impl SplatEngine {
    /// Builds the pipelines and fixed buffers for scenes of at most `capacity` splats. A shader
    /// or pipeline validation error is caught with an error scope.
    pub async fn new(
        core: Rc<GpuCore>,
        shared: SharedSplats,
        capacity: u32,
        scene_format: &'static str,
    ) -> Result<Self, (UnavailableReason, String)> {
        core.device.push_error_scope("validation");
        let compute = SHADER_COMPUTE;
        let layouts = Layouts {
            sort: layout(
                &core,
                vec![
                    uniform_entry(0, compute),
                    storage_entry(1, compute, true),
                    storage_entry(2, compute, false),
                    storage_entry(3, compute, false),
                    storage_entry(4, compute, false),
                    storage_entry(5, compute, false),
                    storage_entry(6, compute, false),
                    storage_entry(7, compute, false),
                ],
            ),
            draw: layout(
                &core,
                vec![
                    uniform_entry(0, SHADER_VERTEX),
                    storage_entry(1, SHADER_VERTEX, true),
                    storage_entry(2, SHADER_VERTEX, true),
                ],
            ),
            resolve: layout(
                &core,
                vec![
                    uniform_entry(0, SHADER_FRAGMENT),
                    data_texture(1, SHADER_FRAGMENT),
                    entry(
                        2,
                        SHADER_FRAGMENT,
                        "texture",
                        Descriptor::new().with("sampleType", "float").build(),
                    ),
                    entry(
                        3,
                        SHADER_FRAGMENT,
                        "sampler",
                        Descriptor::new().with("type", "filtering").build(),
                    ),
                ],
            ),
            decode_chunk: layout(
                &core,
                vec![
                    uniform_entry(0, compute),
                    data_texture(1, compute),
                    data_texture(2, compute),
                    data_texture(3, compute),
                    data_texture(4, compute),
                    data_texture(5, compute),
                    storage_entry(6, compute, true),
                ],
            ),
            decode_global: layout(
                &core,
                vec![
                    uniform_entry(0, compute),
                    storage_entry(1, compute, true),
                    storage_entry(2, compute, false),
                ],
            ),
        };
        let project_source = splat_shaders::project();
        let sort_layout = [&layouts.sort];
        let draw_module = core.device.create_shader_module(
            &Descriptor::new()
                .with("code", splat_shaders::draw().as_str())
                .build(),
        );
        let resolve_module = core.device.create_shader_module(
            &Descriptor::new()
                .with("code", splat_shaders::composite().as_str())
                .build(),
        );
        let render = |module: &JsValue, layout: &JsValue, targets: Vec<JsValue>, depth: bool| {
            let mut descriptor = Descriptor::new()
                .with("layout", core.pipeline_layout(&[layout]))
                .with(
                    "vertex",
                    Descriptor::new()
                        .with("module", module.clone())
                        .with("entryPoint", "vs")
                        .build(),
                )
                .with(
                    "fragment",
                    Descriptor::new()
                        .with("module", module.clone())
                        .with("entryPoint", "fs")
                        .with("targets", array(targets))
                        .build(),
                )
                .with(
                    "primitive",
                    Descriptor::new()
                        .with(
                            "topology",
                            if depth {
                                "triangle-list"
                            } else {
                                "triangle-strip"
                            },
                        )
                        .with("cullMode", "none")
                        .build(),
                );
            if depth {
                descriptor = descriptor.with(
                    "depthStencil",
                    Descriptor::new()
                        .with("format", "depth32float")
                        .with("depthWriteEnabled", true)
                        .with("depthCompare", "always")
                        .build(),
                );
            }
            core.device.create_render_pipeline(&descriptor.build())
        };
        let draw_targets = vec![
            Descriptor::new()
                .with("format", scene_format)
                .with("blend", blend())
                .build(),
            Descriptor::new()
                .with("format", ACCUM_FORMAT)
                .with("blend", blend())
                .build(),
        ];
        let resolve_targets = vec![Descriptor::new().with("format", scene_format).build()];
        let pipelines = Pipelines {
            project: compute_pipeline(&core, &project_source, "project", &sort_layout),
            scan: compute_pipeline(&core, &project_source, "scan", &sort_layout),
            scatter: compute_pipeline(&core, &project_source, "scatter", &sort_layout),
            decode: compute_pipeline(
                &core,
                &splat_shaders::decode(),
                "decode",
                &[&layouts.decode_chunk, &layouts.decode_global],
            ),
            draw: render(
                &draw_module.clone().into(),
                &layouts.draw,
                draw_targets,
                false,
            ),
            resolve: render(
                &resolve_module.clone().into(),
                &layouts.resolve,
                resolve_targets,
                true,
            ),
        };
        let capacity_usize = capacity as usize;
        let uniform = buffer(&core, UNIFORM_BYTES, BUFFER_UNIFORM | BUFFER_COPY_DST);
        let records = buffer(
            &core,
            capacity_usize * RECORD_WORDS * 4,
            BUFFER_STORAGE | BUFFER_COPY_DST,
        );
        let cache = buffer(&core, capacity_usize * RECORD_WORDS * 4, BUFFER_STORAGE);
        let keys = buffer(&core, capacity_usize * 4, BUFFER_STORAGE);
        let order = buffer(&core, capacity_usize * 4, BUFFER_STORAGE);
        let hist = buffer(&core, DEPTH_BIN_COUNT * 4, BUFFER_STORAGE | BUFFER_COPY_DST);
        let counter = buffer(
            &core,
            16,
            BUFFER_STORAGE | BUFFER_COPY_DST | BUFFER_COPY_SRC,
        );
        let args = buffer(
            &core,
            16,
            BUFFER_STORAGE | BUFFER_INDIRECT | BUFFER_COPY_DST,
        );
        let ranges = buffer(&core, MAX_RANGES * 16, BUFFER_STORAGE | BUFFER_COPY_DST);
        let placement = buffer(&core, 32, BUFFER_UNIFORM | BUFFER_COPY_DST);
        let sort_group = group(
            &core,
            &layouts.sort,
            vec![
                buffer_entry(0, &uniform),
                buffer_entry(1, &records),
                buffer_entry(2, &cache),
                buffer_entry(3, &keys),
                buffer_entry(4, &hist),
                buffer_entry(5, &counter),
                buffer_entry(6, &args),
                buffer_entry(7, &order),
            ],
        );
        let draw_group = group(
            &core,
            &layouts.draw,
            vec![
                buffer_entry(0, &uniform),
                buffer_entry(1, &cache),
                buffer_entry(2, &order),
            ],
        );
        let decode_group = group(
            &core,
            &layouts.decode_global,
            vec![
                buffer_entry(0, &placement),
                buffer_entry(1, &ranges),
                buffer_entry(2, &records),
            ],
        );
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
        Ok(Self {
            core,
            shared,
            capacity,
            layouts,
            pipelines,
            uniform,
            records,
            cache,
            keys,
            order,
            hist,
            counter,
            args,
            ranges,
            placement,
            sort_group,
            draw_group,
            decode_group,
            targets: None,
            active_splats: 0,
            depth_threshold: 0.4,
        })
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Splats currently in the record buffer (before culling).
    pub fn active(&self) -> u32 {
        self.active_splats
    }

    pub fn shared(&self) -> &SharedSplats {
        &self.shared
    }

    /// The layouts and chunk-group builder the streaming task needs to make a resident chunk.
    pub fn chunk_layout(&self) -> &JsValue {
        &self.layouts.decode_chunk
    }

    /// The scan's colour and depth-accumulation targets, recreated when their size changes.
    pub fn ensure_targets(&mut self, width: u32, height: u32) {
        if self
            .targets
            .as_ref()
            .is_some_and(|t| t.width == width && t.height == height)
        {
            return;
        }
        if let Some(old) = self.targets.take() {
            old.accum.destroy_texture();
            old.colour.destroy_texture();
        }
        let usage = super::core::COLOUR_TARGET_USAGE;
        let accum = self
            .core
            .create_texture(width, height, ACCUM_FORMAT, usage, 1);
        let colour = self
            .core
            .create_texture(width, height, "rgba8unorm", usage, 1);
        let group = group(
            &self.core,
            &self.layouts.resolve,
            vec![
                buffer_entry(0, &self.uniform),
                Descriptor::new()
                    .with("binding", 1)
                    .with("resource", accum.create_view())
                    .build(),
                Descriptor::new()
                    .with("binding", 2)
                    .with("resource", colour.create_view())
                    .build(),
                Descriptor::new()
                    .with("binding", 3)
                    .with("resource", self.core.sampler_clamp.clone())
                    .build(),
            ],
        );
        self.targets = Some(SplatTargets {
            accum,
            colour,
            group,
            width,
            height,
        });
    }

    /// Views of the scan's colour and accumulation targets, in that order.
    pub fn target_views(&self) -> Option<(JsValue, JsValue)> {
        self.targets
            .as_ref()
            .map(|t| (t.colour.create_view().into(), t.accum.create_view().into()))
    }

    /// Takes any waiting plan or records, writes the frame's uniforms and encodes the decode,
    /// project, sort passes. Returns whether there is anything to draw.
    pub fn prepare(
        &mut self,
        encoder: &GpuCommandEncoder,
        frame: &FrameParams,
        placement: [f32; 8],
        stamps: Option<JsValue>,
    ) -> bool {
        let (plan, records) = {
            let mut shared = self.shared.borrow_mut();
            (shared.plan.take(), shared.records.take())
        };
        if let Some(words) = records {
            let count = (words.len() / RECORD_WORDS).min(self.capacity as usize);
            if count > 0 {
                self.core.queue.write_buffer(
                    &self.records,
                    0.0,
                    &js_sys::Uint32Array::from(&words[..count * RECORD_WORDS]),
                );
            }
            self.active_splats = count as u32;
        }
        if let Some(plan) = plan {
            self.encode_decode(encoder, &plan, placement);
            self.active_splats = plan.total.min(self.capacity);
        }
        if self.active_splats == 0 {
            return false;
        }
        self.write_uniforms(frame);
        encoder.clear_buffer(&self.hist);
        encoder.clear_buffer(&self.counter);
        let pass_descriptor = match stamps {
            Some(stamps) => Descriptor::new().with("timestampWrites", stamps).build(),
            None => Descriptor::new().build(),
        };
        let pass = encoder.begin_compute_pass(&pass_descriptor);
        let groups = self.active_splats.div_ceil(WORKGROUP);
        pass.set_compute_bind_group(0, &self.sort_group);
        pass.set_compute_pipeline(&self.pipelines.project);
        pass.dispatch_workgroups(groups, 1, 1);
        pass.set_compute_pipeline(&self.pipelines.scan);
        pass.dispatch_workgroups(1, 1, 1);
        pass.set_compute_pipeline(&self.pipelines.scatter);
        pass.dispatch_workgroups(groups, 1, 1);
        pass.end_compute();
        true
    }

    fn encode_decode(&self, encoder: &GpuCommandEncoder, plan: &DecodePlan, placement: [f32; 8]) {
        self.core
            .queue
            .write_buffer(&self.placement, 0.0, &float_array(&placement));
        if !plan.ranges.is_empty() {
            let words: Vec<u32> = plan.ranges.iter().flatten().copied().collect();
            self.core.queue.write_buffer(
                &self.ranges,
                0.0,
                &js_sys::Uint32Array::from(words.as_slice()),
            );
        }
        let shared = self.shared.borrow();
        let pass = encoder.begin_compute_pass(&Descriptor::new().build());
        pass.set_compute_pipeline(&self.pipelines.decode);
        pass.set_compute_bind_group(1, &self.decode_group);
        for job in &plan.jobs {
            let Some(chunk) = shared.chunks.get(&job.file) else {
                continue;
            };
            let info = [job.range_base, job.range_count, job.total];
            // The chunk's own uniform keeps its constants and learns this plan's range slice.
            self.core.queue.write_buffer(
                &chunk.params,
                32.0 + 4.0,
                &js_sys::Uint32Array::from(&info[..]),
            );
            pass.set_compute_bind_group(0, &chunk.group);
            pass.dispatch_workgroups(job.total.div_ceil(WORKGROUP), 1, 1);
        }
        pass.end_compute();
    }

    fn write_uniforms(&self, frame: &FrameParams) {
        let mut data: Vec<f32> = Vec::with_capacity(60);
        data.extend_from_slice(&frame.view);
        data.extend_from_slice(&frame.view_proj);
        data.extend_from_slice(&[frame.eye[0], frame.eye[1], frame.eye[2], frame.sort_range_m]);
        data.extend_from_slice(&[
            frame.viewport.0,
            frame.viewport.1,
            frame.focal_px,
            frame.near_cull_m,
        ]);
        data.extend_from_slice(&[
            frame.min_extent_px,
            frame.min_contribution,
            1.0 / 255.0,
            self.active_splats as f32,
        ]);
        data.extend_from_slice(&[
            frame.exposure_gain,
            frame.saturation,
            frame.ambient,
            frame.pool_m,
        ]);
        data.extend_from_slice(&[frame.tint[0], frame.tint[1], frame.tint[2], 0.0]);
        data.extend_from_slice(&[
            frame.pool_centre.0,
            frame.pool_centre.1,
            1.0 / frame.full_size.0,
            1.0 / frame.full_size.1,
        ]);
        data.extend_from_slice(&[
            frame.projection_m10,
            frame.projection_m14,
            frame.depth_bias_m,
            self.depth_threshold,
        ]);
        data.extend_from_slice(&[frame.sky[0], frame.sky[1], frame.sky[2], frame.rim_m]);
        data.extend_from_slice(&frame.clip);
        for index in 0..crate::battle::figure_clear::MAX_FIGURES {
            data.extend_from_slice(&frame.figures.get(index).copied().unwrap_or([0.0; 8]));
        }
        data.extend_from_slice(&frame.fire_light.position_radius);
        data.extend_from_slice(&frame.fire_light.colour_strength);
        self.core
            .queue
            .write_buffer(&self.uniform, 0.0, &float_array(&data));
    }

    /// Copies the number of splats that survived culling into `destination` (16 bytes, copy
    /// destination), for the periodic measurement.
    pub fn copy_visible_count(&self, encoder: &GpuCommandEncoder, destination: &GpuBuffer) {
        encoder.copy_buffer_to_buffer(&self.counter, 0.0, destination, 0.0, 16.0);
    }

    /// Draws the sorted splats into a pass whose targets are the scene colour and the accumulator.
    pub fn draw(&self, pass: &GpuRenderPass) {
        pass.set_pipeline(&self.pipelines.draw);
        pass.set_bind_group(0, &self.draw_group);
        pass.draw_indirect(&self.args, 0.0);
    }

    /// Composites the scan into the scene colour of `pass` and writes its depth into the depth
    /// attachment.
    pub fn resolve(&self, pass: &GpuRenderPass) {
        let Some(targets) = &self.targets else {
            return;
        };
        pass.set_pipeline(&self.pipelines.resolve);
        pass.set_bind_group(0, &targets.group);
        pass.draw(3, 1, 0, 0);
    }
}

impl Drop for SplatEngine {
    fn drop(&mut self) {
        for buffer in [
            &self.uniform,
            &self.records,
            &self.cache,
            &self.keys,
            &self.order,
            &self.hist,
            &self.counter,
            &self.args,
            &self.ranges,
            &self.placement,
        ] {
            buffer.destroy_buffer();
        }
        if let Some(targets) = self.targets.take() {
            targets.accum.destroy_texture();
            targets.colour.destroy_texture();
        }
        self.shared.borrow_mut().chunks.clear();
    }
}

/// Builds a resident chunk from its five decoded plane textures (means low, means high,
/// rotations, scales, colours) and codebooks. The chunk's uniform starts with the constants and
/// leaves room for the per-plan range slice (`info.y`, `info.z`, `info.w`).
pub fn make_chunk(
    core: &GpuCore,
    chunk_layout: &JsValue,
    textures: Vec<GpuTexture>,
    width: u32,
    means_range: ([f32; 3], [f32; 3]),
    codebooks: &[f32],
) -> ChunkGpu {
    let params = buffer(core, CHUNK_PARAM_BYTES, BUFFER_UNIFORM | BUFFER_COPY_DST);
    let (mins, maxs) = means_range;
    let mut constants = vec![
        mins[0], mins[1], mins[2], 0.0, maxs[0], maxs[1], maxs[2], 0.0,
    ];
    constants.extend_from_slice(&[0.0; 4]);
    core.queue
        .write_buffer(&params, 0.0, &float_array(&constants));
    core.queue.write_buffer(
        &params,
        32.0,
        &js_sys::Uint32Array::from(&[width, 0, 0, 0][..]),
    );
    let books = buffer(core, codebooks.len() * 4, BUFFER_STORAGE | BUFFER_COPY_DST);
    core.queue
        .write_buffer(&books, 0.0, &float_array(codebooks));
    let mut entries = vec![buffer_entry(0, &params)];
    for (index, texture) in textures.iter().enumerate() {
        entries.push(
            Descriptor::new()
                .with("binding", index as u32 + 1)
                .with("resource", texture.create_view())
                .build(),
        );
    }
    entries.push(buffer_entry(6, &books));
    let group = group(core, chunk_layout, entries);
    ChunkGpu {
        group,
        params,
        textures,
        codebooks: books,
    }
}
