//! Streams a level-of-detail scan into GPU memory without freezing the page.
//!
//! One task owns the whole load. It fetches `lod-meta.json`, then loads chunk files coarse
//! first: the coarsest level covers the entire scene, so something recognisable draws as soon as
//! the first chunk is resident; finer levels for the play area follow and each arrival replans
//! what is drawn. A chunk is fetched with all five of its planes in flight, decoded by the
//! browser (`createImageBitmap` off the main thread), copied into textures, and the task then
//! yields a frame before the next chunk so uploads never stack up. Decoding the planes into splat
//! records is the GPU's job (see [`splat_engine`](super::splat_engine)); nothing is read back.
//!
//! The task also watches the frame ladder's budget: when it changes the plan is rebuilt from the
//! resident chunks (coarser needs nothing new; finer fetches what is missing).

use std::rc::Rc;

use js_sys::Promise;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use super::{
    core::{GpuCore, IMAGE_USAGE},
    splat_engine::{LoadState, SharedSplats, make_chunk},
    sys::{self, Bitmap, Descriptor, FetchResponse, GpuTexture},
};
use crate::battle::{
    lod::{Focus, Limits, LodIndex, MAX_RANGES},
    math::Vec3,
    splat::{SogMeta, SplatTransform},
};

const MAX_INDEX_BYTES: usize = 16 * 1024 * 1024;
const MAX_CHUNK_META_BYTES: usize = 1024 * 1024;
const MAX_PLANE_SIDE: u32 = 8192;
const BUDGET_POLL_MS: i32 = 250;

/// What to stream and where the detail goes.
pub struct StreamConfig {
    /// URL of `lod-meta.json`; chunk folders sit beside it.
    pub index_url: String,
    pub levels: (u32, u32),
    pub transform: SplatTransform,
    /// The centre of the play area in world space and the radius that gets the finest level.
    pub focus_world: Vec3,
    pub detail_radius_m: f32,
}

#[derive(Debug)]
enum StreamError {
    Fetch,
    Index,
    Chunk,
    Decode,
}

/// Resolves on the next animation frame.
pub async fn next_frame() {
    let promise = Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            // A failed request leaves the promise pending; the task is dropped with the stage.
            let _ = window.request_animation_frame(&resolve);
        }
    });
    let _ = JsFuture::from(promise).await;
}

async fn sleep_ms(milliseconds: i32) {
    let promise = Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                &resolve,
                milliseconds,
            );
        }
    });
    let _ = JsFuture::from(promise).await;
}

async fn fetch_text(url: &str, limit: usize) -> Result<String, StreamError> {
    let response: FetchResponse = JsFuture::from(sys::fetch(url))
        .await
        .map_err(|_| StreamError::Fetch)?
        .unchecked_into();
    if !response.ok() {
        return Err(StreamError::Fetch);
    }
    let text = JsFuture::from(response.text())
        .await
        .map_err(|_| StreamError::Fetch)?
        .as_string()
        .ok_or(StreamError::Fetch)?;
    if text.len() > limit {
        return Err(StreamError::Fetch);
    }
    Ok(text)
}

/// Fetches, decodes and uploads the five planes of one chunk concurrently.
async fn load_planes(
    core: &GpuCore,
    urls: [String; 5],
) -> Result<(Vec<GpuTexture>, u32, u32), StreamError> {
    let responses = Promise::all(&js_sys::Array::from_iter(
        urls.iter().map(|url| JsValue::from(sys::fetch(url))),
    ));
    let responses = JsFuture::from(responses)
        .await
        .map_err(|_| StreamError::Fetch)?;
    let blobs = js_sys::Array::new();
    for response in js_sys::Array::from(&responses).iter() {
        let response: FetchResponse = response.unchecked_into();
        if !response.ok() {
            return Err(StreamError::Fetch);
        }
        blobs.push(&response.blob());
    }
    let blobs = JsFuture::from(Promise::all(&blobs))
        .await
        .map_err(|_| StreamError::Fetch)?;
    // Data planes: no premultiplication and no colour conversion, or the codes would change.
    let options = Descriptor::new()
        .with("premultiplyAlpha", "none")
        .with("colorSpaceConversion", "none")
        .with("imageOrientation", "none")
        .build();
    let bitmaps = js_sys::Array::new();
    for blob in js_sys::Array::from(&blobs).iter() {
        let blob: web_sys::Blob = blob.dyn_into().map_err(|_| StreamError::Decode)?;
        bitmaps.push(&sys::create_image_bitmap(&blob, &options));
    }
    let bitmaps = JsFuture::from(Promise::all(&bitmaps))
        .await
        .map_err(|_| StreamError::Decode)?;
    let mut textures = Vec::with_capacity(5);
    let mut size = None;
    for bitmap in js_sys::Array::from(&bitmaps).iter() {
        let bitmap: Bitmap = bitmap.unchecked_into();
        let (width, height) = (bitmap.width(), bitmap.height());
        let valid = width > 0 && height > 0 && width <= MAX_PLANE_SIDE && height <= MAX_PLANE_SIDE;
        if !valid || *size.get_or_insert((width, height)) != (width, height) {
            bitmap.close();
            textures.iter().for_each(GpuTexture::destroy_texture);
            return Err(StreamError::Decode);
        }
        let texture = core.create_texture(width, height, "rgba8unorm", IMAGE_USAGE, 1);
        core.queue.copy_external_image_to_texture(
            &Descriptor::new().with("source", bitmap.clone()).build(),
            &Descriptor::new()
                .with("texture", texture.clone())
                .with("premultipliedAlpha", false)
                .build(),
            &sys::size_array(width, height),
        );
        bitmap.close();
        textures.push(texture);
    }
    let (width, height) = size.ok_or(StreamError::Decode)?;
    Ok((textures, width, height))
}

fn folder_url(index_url: &str, folder: &str) -> String {
    let base = index_url.rsplit_once('/').map_or("", |(base, _)| base);
    format!("{base}/{folder}")
}

async fn load_chunk(
    core: &GpuCore,
    shared: &SharedSplats,
    chunk_layout: &JsValue,
    index_url: &str,
    folder: &str,
    file: u32,
) -> Result<(), StreamError> {
    let base = folder_url(index_url, folder);
    let meta_text = fetch_text(&format!("{base}/meta.json"), MAX_CHUNK_META_BYTES).await?;
    let meta = SogMeta::parse(&meta_text).map_err(|_| StreamError::Chunk)?;
    let (scales, colours) = meta.codebooks().ok_or(StreamError::Chunk)?;
    let plane = |name: &str| format!("{base}/{name}");
    let urls = [
        plane(&meta.means_files[0]),
        plane(&meta.means_files[1]),
        plane(&meta.quats_file),
        plane(&meta.scales_file),
        plane(&meta.sh0_file),
    ];
    let (textures, width, height) = load_planes(core, urls).await?;
    if (width as usize) * (height as usize) < meta.count {
        textures.iter().for_each(GpuTexture::destroy_texture);
        return Err(StreamError::Chunk);
    }
    let mut books = Vec::with_capacity(512);
    books.extend_from_slice(&scales);
    books.extend_from_slice(&colours);
    let chunk = make_chunk(
        core,
        chunk_layout,
        textures,
        width,
        meta.means_range(),
        &books,
    );
    shared.borrow_mut().chunks.insert(file, chunk);
    Ok(())
}

/// Chunk files in the order to fetch them: first everything at the coarsest level (the whole
/// scene), then what the detailed selection wants, coarse before fine.
fn fetch_order(index: &LodIndex, focus: Focus, limits: Limits) -> Vec<u32> {
    let coarse = Limits {
        finest: limits.coarsest,
        budget: u32::MAX,
        ..limits
    };
    let mut order = index.select(focus, coarse, None).files_coarse_first();
    for file in index.select(focus, limits, None).files_coarse_first() {
        if !order.contains(&file) {
            order.push(file);
        }
    }
    order
}

fn replan(index: &LodIndex, shared: &SharedSplats, focus: Focus, limits: Limits) -> u32 {
    let loaded = {
        let shared = shared.borrow();
        shared.chunks.keys().copied().collect::<std::collections::HashSet<u32>>()
    };
    let resident = |file: u32| loaded.contains(&file);
    let selection = index.select(focus, limits, Some(&resident));
    let plan = selection.plan(MAX_RANGES);
    let total = plan.total;
    let mut shared = shared.borrow_mut();
    shared.stats.splats_planned = total;
    shared.plan = Some(plan);
    total
}

/// Runs the whole streaming load. Spawn it with `wasm_bindgen_futures::spawn_local`; it ends
/// when the scene is complete and the budget has been stable for good, or on failure (the state
/// becomes [`LoadState::Failed`] and the painted ground stays).
pub async fn stream(core: Rc<GpuCore>, shared: SharedSplats, chunk_layout: JsValue, config: StreamConfig) {
    let started = js_sys::Date::now();
    shared.borrow_mut().state = LoadState::Loading;
    let result = run(&core, &shared, &chunk_layout, &config, started).await;
    if result.is_err() {
        // Chunks already resident keep drawing; a scene with nothing resident falls back.
        let mut shared = shared.borrow_mut();
        if shared.chunks.is_empty() {
            shared.state = LoadState::Failed;
        }
    }
}

async fn run(
    core: &GpuCore,
    shared: &SharedSplats,
    chunk_layout: &JsValue,
    config: &StreamConfig,
    started: f64,
) -> Result<(), StreamError> {
    let text = fetch_text(&config.index_url, MAX_INDEX_BYTES).await?;
    let index = LodIndex::parse(&text).map_err(|_| StreamError::Index)?;
    let transform = config.transform;
    let focus = Focus {
        centre: transform.to_scan(config.focus_world),
        radius: config.detail_radius_m / transform.scale,
    };
    let (finest, coarsest) = config.levels;
    let coarsest = coarsest.min(index.level_count().saturating_sub(1) as u32);
    let mut loaded_budget;
    loop {
        let budget = {
            let mut shared = shared.borrow_mut();
            shared.budget_changed = false;
            shared.budget
        };
        loaded_budget = budget;
        let limits = Limits {
            finest,
            coarsest,
            budget,
        };
        let order = fetch_order(&index, focus, limits);
        let wanted = order.len() as u32;
        shared.borrow_mut().stats.chunks_wanted = wanted;
        for file in order {
            if shared.borrow().chunks.contains_key(&file) {
                continue;
            }
            // A new budget mid-load changes what is worth fetching; start over from the plan.
            if shared.borrow().budget != loaded_budget {
                break;
            }
            let folder = index
                .files()
                .get(file as usize)
                .map(|chunk| chunk.folder.clone())
                .ok_or(StreamError::Index)?;
            load_chunk(core, shared, chunk_layout, &config.index_url, &folder, file).await?;
            let planned = replan(&index, shared, focus, limits);
            let now = js_sys::Date::now() - started;
            let mut state = shared.borrow_mut();
            state.stats.chunks_loaded = state.chunks.len() as u32;
            if planned > 0 && state.stats.first_draw_ms.is_none() {
                state.stats.first_draw_ms = Some(now);
                state.state = LoadState::Showing;
            }
            drop(state);
            next_frame().await;
        }
        let settled = {
            let shared = shared.borrow();
            shared.budget == loaded_budget
                && fetch_order(&index, focus, limits)
                    .iter()
                    .all(|file| shared.chunks.contains_key(file))
        };
        if settled {
            replan(&index, shared, focus, limits);
            let mut state = shared.borrow_mut();
            if state.stats.full_detail_ms.is_none() {
                state.stats.full_detail_ms = Some(js_sys::Date::now() - started);
            }
            state.state = LoadState::Complete;
            drop(state);
            // Stay alive for later budget changes: the ladder may raise or lower the budget.
            while shared.borrow().budget == loaded_budget {
                sleep_ms(BUDGET_POLL_MS).await;
                if Rc::strong_count(shared) == 1 {
                    return Ok(());
                }
            }
        }
    }
}
