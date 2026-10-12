//! Rust declarations for the WebGPU calls the battlefield uses, and a descriptor builder.
//!
//! web-sys's `Gpu*` bindings need the global `--cfg=web_sys_unstable_apis` build flag, which would
//! change every crate's WebAssembly build. These declarations are plain `wasm_bindgen` imports of
//! the stable browser API, limited to what the renderer calls. They are binding code, not
//! application logic: descriptors are ordinary JavaScript objects assembled from Rust by
//! [`Descriptor`], and the object types are only ever created by the browser (so they are cast with
//! `unchecked_into`, never `dyn_into`).

use js_sys::{Object, Reflect};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPU)]
    pub type Gpu;
    #[wasm_bindgen(method, js_name = requestAdapter)]
    pub fn request_adapter(this: &Gpu) -> js_sys::Promise;
    #[wasm_bindgen(method, js_name = getPreferredCanvasFormat)]
    pub fn preferred_canvas_format(this: &Gpu) -> String;

    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPUAdapter)]
    pub type GpuAdapter;
    #[wasm_bindgen(method, js_name = requestDevice)]
    pub fn request_device(this: &GpuAdapter, descriptor: &JsValue) -> js_sys::Promise;
    #[wasm_bindgen(method, getter)]
    pub fn info(this: &GpuAdapter) -> JsValue;

    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPUDevice)]
    pub type GpuDevice;
    #[wasm_bindgen(method, getter)]
    pub fn queue(this: &GpuDevice) -> GpuQueue;
    #[wasm_bindgen(method, getter)]
    pub fn lost(this: &GpuDevice) -> js_sys::Promise;
    #[wasm_bindgen(method, js_name = createBuffer)]
    pub fn create_buffer(this: &GpuDevice, descriptor: &JsValue) -> GpuBuffer;
    #[wasm_bindgen(method, js_name = createTexture)]
    pub fn create_texture(this: &GpuDevice, descriptor: &JsValue) -> GpuTexture;
    #[wasm_bindgen(method, js_name = createSampler)]
    pub fn create_sampler(this: &GpuDevice, descriptor: &JsValue) -> GpuSampler;
    #[wasm_bindgen(method, js_name = createBindGroupLayout)]
    pub fn create_bind_group_layout(this: &GpuDevice, descriptor: &JsValue) -> JsValue;
    #[wasm_bindgen(method, js_name = createPipelineLayout)]
    pub fn create_pipeline_layout(this: &GpuDevice, descriptor: &JsValue) -> JsValue;
    #[wasm_bindgen(method, js_name = createShaderModule)]
    pub fn create_shader_module(this: &GpuDevice, descriptor: &JsValue) -> GpuShaderModule;
    #[wasm_bindgen(method, js_name = createRenderPipeline)]
    pub fn create_render_pipeline(this: &GpuDevice, descriptor: &JsValue) -> GpuRenderPipeline;
    #[wasm_bindgen(method, js_name = createQuerySet)]
    pub fn create_query_set(this: &GpuDevice, descriptor: &JsValue) -> GpuQuerySet;
    #[wasm_bindgen(method, js_name = createComputePipeline)]
    pub fn create_compute_pipeline(this: &GpuDevice, descriptor: &JsValue) -> GpuComputePipeline;
    #[wasm_bindgen(method, js_name = createBindGroup)]
    pub fn create_bind_group(this: &GpuDevice, descriptor: &JsValue) -> GpuBindGroup;
    #[wasm_bindgen(method, js_name = createCommandEncoder)]
    pub fn create_command_encoder(this: &GpuDevice) -> GpuCommandEncoder;
    #[wasm_bindgen(method, js_name = pushErrorScope)]
    pub fn push_error_scope(this: &GpuDevice, filter: &str);
    #[wasm_bindgen(method, js_name = popErrorScope)]
    pub fn pop_error_scope(this: &GpuDevice) -> js_sys::Promise;
    #[wasm_bindgen(method)]
    pub fn destroy(this: &GpuDevice);

    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPUQueue)]
    pub type GpuQueue;
    #[wasm_bindgen(method)]
    pub fn submit(this: &GpuQueue, commands: &js_sys::Array);
    #[wasm_bindgen(method, js_name = writeBuffer)]
    pub fn write_buffer(this: &GpuQueue, buffer: &GpuBuffer, offset: f64, data: &Object);
    #[wasm_bindgen(method, js_name = writeTexture)]
    pub fn write_texture(
        this: &GpuQueue,
        destination: &JsValue,
        data: &Object,
        layout: &JsValue,
        size: &JsValue,
    );
    #[wasm_bindgen(method, js_name = onSubmittedWorkDone)]
    pub fn on_submitted_work_done(this: &GpuQueue) -> js_sys::Promise;
    #[wasm_bindgen(method, js_name = copyExternalImageToTexture)]
    pub fn copy_external_image_to_texture(
        this: &GpuQueue,
        source: &JsValue,
        destination: &JsValue,
        size: &JsValue,
    );

    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPUBuffer)]
    pub type GpuBuffer;
    #[wasm_bindgen(method, js_name = mapAsync)]
    pub fn map_async(this: &GpuBuffer, mode: u32) -> js_sys::Promise;
    #[wasm_bindgen(method, js_name = getMappedRange)]
    pub fn get_mapped_range(this: &GpuBuffer) -> js_sys::ArrayBuffer;
    #[wasm_bindgen(method)]
    pub fn unmap(this: &GpuBuffer);
    #[wasm_bindgen(method, js_name = destroy)]
    pub fn destroy_buffer(this: &GpuBuffer);

    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPUTexture)]
    pub type GpuTexture;
    #[wasm_bindgen(method, js_name = createView)]
    pub fn create_view(this: &GpuTexture) -> GpuTextureView;
    #[wasm_bindgen(method, js_name = destroy)]
    pub fn destroy_texture(this: &GpuTexture);

    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPUTextureView)]
    pub type GpuTextureView;
    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPUSampler)]
    pub type GpuSampler;
    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPUShaderModule)]
    pub type GpuShaderModule;
    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPUBindGroup)]
    pub type GpuBindGroup;
    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPUCommandBuffer)]
    pub type GpuCommandBuffer;

    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPURenderPipeline)]
    pub type GpuRenderPipeline;
    #[wasm_bindgen(method, js_name = getBindGroupLayout)]
    pub fn get_bind_group_layout(this: &GpuRenderPipeline, index: u32) -> JsValue;

    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPUQuerySet)]
    pub type GpuQuerySet;

    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPUComputePipeline)]
    pub type GpuComputePipeline;
    #[wasm_bindgen(method, js_name = getBindGroupLayout)]
    pub fn get_compute_bind_group_layout(this: &GpuComputePipeline, index: u32) -> JsValue;

    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPUComputePassEncoder)]
    pub type GpuComputePass;
    #[wasm_bindgen(method, js_name = setPipeline)]
    pub fn set_compute_pipeline(this: &GpuComputePass, pipeline: &GpuComputePipeline);
    #[wasm_bindgen(method, js_name = setBindGroup)]
    pub fn set_compute_bind_group(this: &GpuComputePass, index: u32, group: &GpuBindGroup);
    #[wasm_bindgen(method, js_name = dispatchWorkgroups)]
    pub fn dispatch_workgroups(this: &GpuComputePass, x: u32, y: u32, z: u32);
    #[wasm_bindgen(method, js_name = end)]
    pub fn end_compute(this: &GpuComputePass);

    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPUCommandEncoder)]
    pub type GpuCommandEncoder;
    #[wasm_bindgen(method, js_name = beginRenderPass)]
    pub fn begin_render_pass(this: &GpuCommandEncoder, descriptor: &JsValue) -> GpuRenderPass;
    #[wasm_bindgen(method, js_name = beginComputePass)]
    pub fn begin_compute_pass(this: &GpuCommandEncoder, descriptor: &JsValue) -> GpuComputePass;
    #[wasm_bindgen(method, js_name = resolveQuerySet)]
    pub fn resolve_query_set(
        this: &GpuCommandEncoder,
        query_set: &GpuQuerySet,
        first: u32,
        count: u32,
        destination: &GpuBuffer,
        offset: f64,
    );
    #[wasm_bindgen(method, js_name = clearBuffer)]
    pub fn clear_buffer(this: &GpuCommandEncoder, buffer: &GpuBuffer);
    #[wasm_bindgen(method, js_name = copyBufferToBuffer)]
    pub fn copy_buffer_to_buffer(
        this: &GpuCommandEncoder,
        source: &GpuBuffer,
        source_offset: f64,
        destination: &GpuBuffer,
        destination_offset: f64,
        size: f64,
    );
    #[wasm_bindgen(method, js_name = copyTextureToBuffer)]
    pub fn copy_texture_to_buffer(
        this: &GpuCommandEncoder,
        source: &JsValue,
        destination: &JsValue,
        size: &JsValue,
    );
    #[wasm_bindgen(method)]
    pub fn finish(this: &GpuCommandEncoder) -> GpuCommandBuffer;

    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPURenderPassEncoder)]
    pub type GpuRenderPass;
    #[wasm_bindgen(method, js_name = setPipeline)]
    pub fn set_pipeline(this: &GpuRenderPass, pipeline: &GpuRenderPipeline);
    #[wasm_bindgen(method, js_name = setBindGroup)]
    pub fn set_bind_group(this: &GpuRenderPass, index: u32, group: &GpuBindGroup);
    #[wasm_bindgen(method, js_name = setVertexBuffer)]
    pub fn set_vertex_buffer(
        this: &GpuRenderPass,
        slot: u32,
        buffer: &GpuBuffer,
        offset: f64,
        size: f64,
    );
    #[wasm_bindgen(method, js_name = drawIndirect)]
    pub fn draw_indirect(this: &GpuRenderPass, buffer: &GpuBuffer, offset: f64);
    #[wasm_bindgen(method)]
    pub fn draw(
        this: &GpuRenderPass,
        vertex_count: u32,
        instance_count: u32,
        first_vertex: u32,
        first_instance: u32,
    );
    #[wasm_bindgen(method)]
    pub fn end(this: &GpuRenderPass);

    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = GPUCanvasContext)]
    pub type GpuCanvasContext;
    #[wasm_bindgen(method)]
    pub fn configure(this: &GpuCanvasContext, configuration: &JsValue);
    #[wasm_bindgen(method)]
    pub fn unconfigure(this: &GpuCanvasContext);
    #[wasm_bindgen(method, js_name = getCurrentTexture)]
    pub fn current_texture(this: &GpuCanvasContext) -> GpuTexture;
}

pub const BUFFER_MAP_READ: u32 = 0x0001;
pub const SHADER_VERTEX: u32 = 0x1;
pub const SHADER_FRAGMENT: u32 = 0x2;
pub const SHADER_COMPUTE: u32 = 0x4;
pub const BUFFER_INDIRECT: u32 = 0x0100;
pub const BUFFER_QUERY_RESOLVE: u32 = 0x0200;
pub const BUFFER_COPY_SRC: u32 = 0x0004;
pub const BUFFER_COPY_DST: u32 = 0x0008;
pub const BUFFER_UNIFORM: u32 = 0x0040;
pub const BUFFER_STORAGE: u32 = 0x0080;
pub const BUFFER_VERTEX: u32 = 0x0020;
pub const TEXTURE_COPY_SRC: u32 = 0x01;
pub const TEXTURE_COPY_DST: u32 = 0x02;
pub const TEXTURE_BINDING: u32 = 0x04;
pub const TEXTURE_RENDER_ATTACHMENT: u32 = 0x10;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = createImageBitmap)]
    pub fn create_image_bitmap(source: &web_sys::Blob, options: &JsValue) -> js_sys::Promise;
    #[wasm_bindgen(js_name = fetch)]
    pub fn fetch(url: &str) -> js_sys::Promise;

    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = Response)]
    pub type FetchResponse;
    #[wasm_bindgen(method, getter)]
    pub fn ok(this: &FetchResponse) -> bool;
    #[wasm_bindgen(method)]
    pub fn blob(this: &FetchResponse) -> js_sys::Promise;
    #[wasm_bindgen(method, js_name = arrayBuffer)]
    pub fn array_buffer(this: &FetchResponse) -> js_sys::Promise;
    #[wasm_bindgen(method)]
    pub fn text(this: &FetchResponse) -> js_sys::Promise;

    #[derive(Clone)]
    #[wasm_bindgen(extends = Object, js_name = ImageBitmap)]
    pub type Bitmap;
    #[wasm_bindgen(method, getter)]
    pub fn width(this: &Bitmap) -> u32;
    #[wasm_bindgen(method, getter)]
    pub fn height(this: &Bitmap) -> u32;
    #[wasm_bindgen(method)]
    pub fn close(this: &Bitmap);
}

/// A JavaScript object built key by key. Setting a property on a fresh plain object cannot fail,
/// so the `Reflect::set` result is checked only in debug builds.
pub struct Descriptor(Object);

impl Descriptor {
    pub fn new() -> Self {
        Self(Object::new())
    }

    pub fn with(self, key: &str, value: impl Into<JsValue>) -> Self {
        let result = Reflect::set(&self.0, &JsValue::from_str(key), &value.into());
        debug_assert!(result.is_ok());
        self
    }

    pub fn build(self) -> JsValue {
        self.0.into()
    }
}

pub fn array(items: impl IntoIterator<Item = JsValue>) -> js_sys::Array {
    let array = js_sys::Array::new();
    for item in items {
        array.push(&item);
    }
    array
}

pub fn float_array(values: &[f32]) -> js_sys::Float32Array {
    js_sys::Float32Array::from(values)
}

pub fn size_array(width: u32, height: u32) -> JsValue {
    array([JsValue::from(width), JsValue::from(height)]).into()
}
