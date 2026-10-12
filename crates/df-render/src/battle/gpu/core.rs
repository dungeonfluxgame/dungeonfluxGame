//! Device acquisition and the objects every pass shares. Each way WebGPU can be absent has its own
//! [`UnavailableReason`]; none of them retries. The caller keeps the flat combat view.

use std::{cell::RefCell, rc::Rc};

use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::HtmlCanvasElement;

use super::sys::{
    self, Descriptor, Gpu, GpuAdapter, GpuBindGroup, GpuCanvasContext, GpuDevice, GpuQueue,
    GpuSampler, GpuTexture, SHADER_FRAGMENT, SHADER_VERTEX, TEXTURE_BINDING, TEXTURE_COPY_DST,
    TEXTURE_COPY_SRC, TEXTURE_RENDER_ATTACHMENT, array,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnavailableReason {
    NoWebGpu,
    NoAdapter,
    NoDevice,
    CanvasContext,
    CanvasFormat,
    ShaderError,
    DeviceLost,
    NotMounted,
}

impl UnavailableReason {
    /// Stable text for the `data-battlefield="unavailable:<reason>"` attribute.
    pub fn key(self) -> &'static str {
        match self {
            Self::NoWebGpu => "no-webgpu",
            Self::NoAdapter => "no-adapter",
            Self::NoDevice => "no-device",
            Self::CanvasContext => "no-canvas-context",
            Self::CanvasFormat => "canvas-format",
            Self::ShaderError => "shader-error",
            Self::DeviceLost => "device-lost",
            Self::NotMounted => "not-mounted",
        }
    }
}

pub struct GpuCore {
    pub device: GpuDevice,
    pub queue: GpuQueue,
    pub context: GpuCanvasContext,
    pub format: &'static str,
    pub adapter_description: String,
    pub layout_frame: JsValue,
    pub layout_texture: JsValue,
    pub layout_splat: JsValue,
    pub layout_post: JsValue,
    pub sampler_repeat: GpuSampler,
    pub sampler_clamp: GpuSampler,
    lost: Rc<RefCell<Option<String>>>,
    first_error: Rc<RefCell<Option<String>>>,
    error_handler: Closure<dyn FnMut(JsValue)>,
}

fn property(target: &JsValue, key: &str) -> Option<JsValue> {
    js_sys::Reflect::get(target, &JsValue::from_str(key))
        .ok()
        .filter(|value| !value.is_undefined() && !value.is_null())
}

fn text(value: &JsValue, key: &str) -> String {
    property(value, key)
        .and_then(|value| value.as_string())
        .unwrap_or_default()
}

fn layout_entry(binding: u32, visibility: u32, kind: &str, config: JsValue) -> JsValue {
    Descriptor::new()
        .with("binding", binding)
        .with("visibility", visibility)
        .with(kind, config)
        .build()
}

fn bind_group_layout(device: &GpuDevice, entries: Vec<JsValue>) -> JsValue {
    device.create_bind_group_layout(&Descriptor::new().with("entries", array(entries)).build())
}

pub async fn acquire(canvas: &HtmlCanvasElement) -> Result<GpuCore, UnavailableReason> {
    let navigator =
        property(&js_sys::global().into(), "navigator").ok_or(UnavailableReason::NoWebGpu)?;
    let gpu: Gpu = property(&navigator, "gpu")
        .ok_or(UnavailableReason::NoWebGpu)?
        .unchecked_into();
    let adapter = JsFuture::from(gpu.request_adapter())
        .await
        .ok()
        .filter(|adapter| !adapter.is_null() && !adapter.is_undefined())
        .ok_or(UnavailableReason::NoAdapter)?
        .unchecked_into::<GpuAdapter>();
    let info = adapter.info();
    let adapter_description = format!("{}/{}", text(&info, "vendor"), text(&info, "architecture"));
    let device: GpuDevice = JsFuture::from(adapter.request_device(&Descriptor::new().build()))
        .await
        .map_err(|_| UnavailableReason::NoDevice)?
        .unchecked_into();
    let context: GpuCanvasContext = canvas
        .get_context("webgpu")
        .ok()
        .flatten()
        .ok_or(UnavailableReason::CanvasContext)?
        .unchecked_into();
    let format = match gpu.preferred_canvas_format().as_str() {
        "bgra8unorm" => "bgra8unorm",
        "rgba8unorm" => "rgba8unorm",
        _ => return Err(UnavailableReason::CanvasFormat),
    };
    context.configure(
        &Descriptor::new()
            .with("device", device.clone())
            .with("format", format)
            .with("alphaMode", "opaque")
            .build(),
    );

    let lost = Rc::new(RefCell::new(None));
    {
        let lost = lost.clone();
        let promise = device.lost();
        wasm_bindgen_futures::spawn_local(async move {
            let reason = match JsFuture::from(promise).await {
                Ok(info) => {
                    let reason = text(&info, "reason");
                    if reason.is_empty() {
                        "lost".to_owned()
                    } else {
                        reason
                    }
                }
                Err(_) => "lost".to_owned(),
            };
            *lost.borrow_mut() = Some(reason);
        });
    }
    let first_error = Rc::new(RefCell::new(None));
    let error_handler = {
        let first_error = first_error.clone();
        Closure::wrap(Box::new(move |event: JsValue| {
            let message = property(&event, "error")
                .map(|error| text(&error, "message"))
                .unwrap_or_default();
            first_error.borrow_mut().get_or_insert(message);
        }) as Box<dyn FnMut(JsValue)>)
    };
    // Setting a handler property on the device object cannot fail on a live device.
    let _ = js_sys::Reflect::set(
        &device,
        &JsValue::from_str("onuncapturederror"),
        error_handler.as_ref(),
    );

    let both = SHADER_VERTEX | SHADER_FRAGMENT;
    let layout_frame = bind_group_layout(
        &device,
        vec![layout_entry(
            0,
            both,
            "buffer",
            Descriptor::new().with("type", "uniform").build(),
        )],
    );
    let layout_texture = bind_group_layout(
        &device,
        vec![
            layout_entry(
                0,
                both,
                "texture",
                Descriptor::new().with("sampleType", "float").build(),
            ),
            layout_entry(
                1,
                both,
                "sampler",
                Descriptor::new().with("type", "filtering").build(),
            ),
        ],
    );
    let layout_splat = bind_group_layout(
        &device,
        vec![
            layout_entry(
                0,
                SHADER_VERTEX,
                "buffer",
                Descriptor::new().with("type", "read-only-storage").build(),
            ),
            layout_entry(
                1,
                SHADER_VERTEX,
                "buffer",
                Descriptor::new().with("type", "read-only-storage").build(),
            ),
        ],
    );
    let layout_post = bind_group_layout(
        &device,
        vec![
            layout_entry(
                0,
                both,
                "buffer",
                Descriptor::new().with("type", "uniform").build(),
            ),
            layout_entry(
                1,
                both,
                "texture",
                Descriptor::new().with("sampleType", "float").build(),
            ),
            layout_entry(
                2,
                both,
                "sampler",
                Descriptor::new().with("type", "filtering").build(),
            ),
        ],
    );
    let sampler = |address: &str| {
        device.create_sampler(
            &Descriptor::new()
                .with("addressModeU", address)
                .with("addressModeV", address)
                .with("magFilter", "linear")
                .with("minFilter", "linear")
                .with("mipmapFilter", "linear")
                .with("maxAnisotropy", 8)
                .build(),
        )
    };
    let sampler_repeat = sampler("repeat");
    let sampler_clamp = sampler("clamp-to-edge");
    let queue = device.queue();
    Ok(GpuCore {
        device,
        queue,
        context,
        format,
        adapter_description,
        layout_frame,
        layout_texture,
        layout_splat,
        layout_post,
        sampler_repeat,
        sampler_clamp,
        lost,
        first_error,
        error_handler,
    })
}

impl GpuCore {
    /// The device-loss reason once the device is gone.
    pub fn lost(&self) -> Option<String> {
        self.lost.borrow().clone()
    }

    pub fn first_error(&self) -> Option<String> {
        self.first_error.borrow().clone()
    }

    pub fn pipeline_layout(&self, groups: &[&JsValue]) -> JsValue {
        let layouts = array(groups.iter().map(|group| (*group).clone()));
        self.device
            .create_pipeline_layout(&Descriptor::new().with("bindGroupLayouts", layouts).build())
    }

    pub fn texture_bind_group(&self, texture: &GpuTexture, clamp: bool) -> GpuBindGroup {
        let sampler = if clamp {
            &self.sampler_clamp
        } else {
            &self.sampler_repeat
        };
        self.device.create_bind_group(
            &Descriptor::new()
                .with("layout", self.layout_texture.clone())
                .with(
                    "entries",
                    array([
                        Descriptor::new()
                            .with("binding", 0)
                            .with("resource", texture.create_view())
                            .build(),
                        Descriptor::new()
                            .with("binding", 1)
                            .with("resource", sampler.clone())
                            .build(),
                    ]),
                )
                .build(),
        )
    }

    pub fn create_texture(
        &self,
        width: u32,
        height: u32,
        format: &str,
        usage: u32,
        mips: u32,
    ) -> GpuTexture {
        self.device.create_texture(
            &Descriptor::new()
                .with("size", sys::size_array(width, height))
                .with("format", format)
                .with("usage", usage)
                .with("mipLevelCount", mips)
                .build(),
        )
    }
}

impl Drop for GpuCore {
    fn drop(&mut self) {
        // Removing the handler stops the closure from being called after it is freed.
        let _ = js_sys::Reflect::set(
            &self.device,
            &JsValue::from_str("onuncapturederror"),
            &JsValue::NULL,
        );
        let _ = &self.error_handler;
        self.context.unconfigure();
        self.device.destroy();
    }
}

pub const COLOUR_TARGET_USAGE: u32 = TEXTURE_RENDER_ATTACHMENT | TEXTURE_BINDING;
pub const IMAGE_USAGE: u32 =
    TEXTURE_BINDING | TEXTURE_COPY_DST | TEXTURE_RENDER_ATTACHMENT | TEXTURE_COPY_SRC;
