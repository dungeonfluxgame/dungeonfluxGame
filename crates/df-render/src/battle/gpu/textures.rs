//! Image and byte loading for the battlefield. Pictures are decoded by the browser
//! (`createImageBitmap`) and copied to GPU textures with `copyExternalImageToTexture`, never
//! through a 2D canvas, so premultiplication and colour conversion are exactly what is asked for.
//! Exact data planes (Gaussian splat SOG bundles) use premultiply NONE and colour conversion NONE
//! and are read back with `copyTextureToBuffer` + `mapAsync`.

use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use super::{
    core::{GpuCore, IMAGE_USAGE},
    sys::{
        self, BUFFER_COPY_DST, BUFFER_MAP_READ, Bitmap, Descriptor, FetchResponse, GpuBindGroup,
        GpuTexture, TEXTURE_COPY_SRC, array,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextureError {
    Fetch,
    Decode,
    TooLarge,
    Readback,
}

pub struct ImageTexture {
    pub texture: GpuTexture,
    pub bind_group: GpuBindGroup,
    pub width: u32,
    pub height: u32,
}

impl Drop for ImageTexture {
    fn drop(&mut self) {
        self.texture.destroy_texture();
    }
}

const MAX_MIPS: u32 = 11;

async fn response(url: &str) -> Result<FetchResponse, TextureError> {
    let value = JsFuture::from(sys::fetch(url))
        .await
        .map_err(|_| TextureError::Fetch)?;
    let response: FetchResponse = value.unchecked_into();
    if !response.ok() {
        return Err(TextureError::Fetch);
    }
    Ok(response)
}

pub async fn fetch_blob(url: &str) -> Result<web_sys::Blob, TextureError> {
    let blob = JsFuture::from(response(url).await?.blob())
        .await
        .map_err(|_| TextureError::Fetch)?;
    blob.dyn_into().map_err(|_| TextureError::Fetch)
}

pub async fn fetch_bytes(url: &str, limit: usize) -> Result<Vec<u8>, TextureError> {
    let buffer = JsFuture::from(response(url).await?.array_buffer())
        .await
        .map_err(|_| TextureError::Fetch)?;
    let bytes = Uint8Array::new(&buffer);
    if bytes.length() as usize > limit {
        return Err(TextureError::TooLarge);
    }
    Ok(bytes.to_vec())
}

pub async fn fetch_text(url: &str, limit: usize) -> Result<String, TextureError> {
    let bytes = fetch_bytes(url, limit).await?;
    String::from_utf8(bytes).map_err(|_| TextureError::Decode)
}

async fn bitmap(
    blob: &web_sys::Blob,
    premultiplied: bool,
    size: Option<(u32, u32)>,
) -> Result<Bitmap, TextureError> {
    let mut options = Descriptor::new()
        .with(
            "premultiplyAlpha",
            if premultiplied { "premultiply" } else { "none" },
        )
        .with("colorSpaceConversion", "none")
        .with("imageOrientation", "none");
    if let Some((width, height)) = size {
        options = options
            .with("resizeWidth", width)
            .with("resizeHeight", height)
            .with("resizeQuality", "high");
    }
    let value = JsFuture::from(sys::create_image_bitmap(blob, &options.build()))
        .await
        .map_err(|_| TextureError::Decode)?;
    Ok(value.unchecked_into())
}

fn copy_bitmap(
    core: &GpuCore,
    bitmap: &Bitmap,
    texture: &GpuTexture,
    level: u32,
    premultiplied: bool,
    width: u32,
    height: u32,
) {
    core.queue.copy_external_image_to_texture(
        &Descriptor::new().with("source", bitmap.clone()).build(),
        &Descriptor::new()
            .with("texture", texture.clone())
            .with("mipLevel", level)
            .with("premultipliedAlpha", premultiplied)
            .build(),
        &sys::size_array(width, height),
    );
}

/// Loads a picture as a mip-mapped texture. `premultiplied` pictures (cut-outs with transparency)
/// get their alpha multiplied in by the decoder, so every mip level blends without dark fringes.
pub async fn load_image(
    core: &GpuCore,
    url: &str,
    premultiplied: bool,
    repeat: bool,
) -> Result<ImageTexture, TextureError> {
    let blob = fetch_blob(url).await?;
    let base = bitmap(&blob, premultiplied, None).await?;
    let (width, height) = (base.width(), base.height());
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        base.close();
        return Err(TextureError::Decode);
    }
    let levels = (32 - width.max(height).leading_zeros()).min(MAX_MIPS);
    let texture = core.create_texture(width, height, "rgba8unorm", IMAGE_USAGE, levels);
    copy_bitmap(core, &base, &texture, 0, premultiplied, width, height);
    base.close();
    for level in 1..levels {
        let (w, h) = ((width >> level).max(1), (height >> level).max(1));
        let reduced = bitmap(&blob, premultiplied, Some((w, h))).await?;
        copy_bitmap(core, &reduced, &texture, level, premultiplied, w, h);
        reduced.close();
    }
    let bind_group = core.texture_bind_group(&texture, !repeat);
    Ok(ImageTexture {
        texture,
        bind_group,
        width,
        height,
    })
}

/// A one-pixel texture, used while a real picture loads or when it is missing.
pub fn solid(core: &GpuCore, rgba: [u8; 4]) -> ImageTexture {
    let texture = core.create_texture(1, 1, "rgba8unorm", IMAGE_USAGE, 1);
    core.queue.write_texture(
        &Descriptor::new().with("texture", texture.clone()).build(),
        &Uint8Array::from(&rgba[..]),
        &Descriptor::new()
            .with("bytesPerRow", 4)
            .with("rowsPerImage", 1)
            .build(),
        &sys::size_array(1, 1),
    );
    let bind_group = core.texture_bind_group(&texture, false);
    ImageTexture {
        texture,
        bind_group,
        width: 1,
        height: 1,
    }
}

/// Decodes `url` with premultiply NONE and colour-space conversion NONE and returns its exact
/// RGBA bytes plus size. A SOG plane carries data, so no pixel may be altered on the way.
pub async fn read_exact_rgba(
    core: &GpuCore,
    url: &str,
) -> Result<(Vec<u8>, u32, u32), TextureError> {
    let blob = fetch_blob(url).await?;
    let image = bitmap(&blob, false, None).await?;
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        image.close();
        return Err(TextureError::Decode);
    }
    let texture = core.create_texture(
        width,
        height,
        "rgba8unorm",
        IMAGE_USAGE | TEXTURE_COPY_SRC,
        1,
    );
    copy_bitmap(core, &image, &texture, 0, false, width, height);
    image.close();
    let bytes_per_row = (width * 4).next_multiple_of(256);
    let size = bytes_per_row as f64 * height as f64;
    let readback = core.device.create_buffer(
        &Descriptor::new()
            .with("size", size)
            .with("usage", BUFFER_COPY_DST | BUFFER_MAP_READ)
            .build(),
    );
    let encoder = core.device.create_command_encoder();
    encoder.copy_texture_to_buffer(
        &Descriptor::new().with("texture", texture.clone()).build(),
        &Descriptor::new()
            .with("buffer", readback.clone())
            .with("bytesPerRow", bytes_per_row)
            .with("rowsPerImage", height)
            .build(),
        &sys::size_array(width, height),
    );
    core.queue.submit(&array([encoder.finish().into()]));
    let mapped = JsFuture::from(readback.map_async(BUFFER_MAP_READ)).await;
    let result = match mapped {
        Ok(_) => {
            let padded = Uint8Array::new(&readback.get_mapped_range()).to_vec();
            let mut rows = Vec::with_capacity((width * height * 4) as usize);
            for row in 0..height as usize {
                let start = row * bytes_per_row as usize;
                rows.extend_from_slice(&padded[start..start + (width * 4) as usize]);
            }
            Ok((rows, width, height))
        }
        Err(_) => Err(TextureError::Readback),
    };
    readback.unmap();
    readback.destroy_buffer();
    texture.destroy_texture();
    result
}
