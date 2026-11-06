//! Lossy WebP encoding through the pinned native encoder.
//!
//! Owns the `WebPConfig` / `WebPPicture` / `WebPEncode` call for
//! `Encoding::WebP` with a quality: bounded writer, progress hook, and RAII
//! picture free on every exit. It never spawns work, never touches the
//! network or filesystem, and never outlives the caller's reservation: the
//! native call runs synchronously on the calling thread.
//!
//! How it fails: refusals stay typed (`Cancelled`, `OutputTooLarge`,
//! `EncodeFailed`); the writer stops at the output limit instead of growing
//! past it; a panic can never cross into native code because both callbacks
//! contain it and report failure instead.

use std::ffi::c_void;
use std::os::raw::c_int;
use std::panic::AssertUnwindSafe;

use image::{RgbImage, RgbaImage};

use super::RasterError;
use super::bounds::LiveBudget;
use crate::output::LossyQuality;

/// Revision of the lossy WebP encoder settings below. A change that can
/// alter output bytes (library, preset, fields, import path) takes a new
/// revision so the recipe key can version it.
pub(crate) const WEBP_ENCODER_REVISION: u32 = 2;

/// Pinned encoder source with the exact selected fields. Raw alpha keeps
/// exact coverage without nested lossless compression; output bytes differ
/// from revision 1 on alpha-bearing inputs and may be larger.
pub(crate) const WEBP_ENCODER_SOURCE: &str = "libwebp 1.5.0 static via \
    libwebp-sys2 0.2.0 / 0.1.11; method 4, thread_level 0, low_memory 1, \
    lossless 0, alpha_compression 0, alpha_quality 100";

/// Per-axis native dimension ceiling in pixels.
const MAX_NATIVE_DIMENSION_PX: u32 = libwebp_sys::WEBP_MAX_DIMENSION as u32;

/// Borrowed cancellation check with the caller's lifetime. The raw pointer
/// in state never outlives the synchronous encode that owns both.
type CancelCallback<'a> = dyn Fn() -> Result<(), RasterError> + 'a;

/// Shared mutable state for the native writer and progress callbacks. It
/// lives on the caller's stack across one synchronous `WebPEncode`, so no
/// callback outlives it and no work continues after the return.
struct CallbackState<'a> {
    bytes: Vec<u8>,
    limit_bytes: u64,
    check_cancel: *const CancelCallback<'a>,
    cancelled: bool,
    over_limit: bool,
    allocation_failed: bool,
}

mod memory;

/// Conservative requested heap bytes for the pinned native path and Rust
/// writer, excluding caller images, allocator metadata and codec stack.
pub(crate) fn estimate_native_peak_bytes(
    width_px: u32,
    height_px: u32,
    has_alpha: bool,
    max_output_bytes: u64,
) -> u64 {
    memory::estimate_peak_bytes(width_px, height_px, has_alpha, max_output_bytes)
}

/// Encodes RGBA pixels as lossy WebP with raw exact alpha. Translucent
/// coverage round-trips exactly; translucent color stays lossy.
pub(crate) fn encode_lossy_rgba(
    image: &RgbaImage,
    quality: LossyQuality,
    budget: &LiveBudget,
    check_cancel: super::CancelCheck<'_>,
    retained_bytes: u64,
) -> Result<Vec<u8>, RasterError> {
    let (width_px, height_px) = image.dimensions();
    check_native_dimensions(width_px, height_px)?;
    reserve_native(width_px, height_px, retained_bytes, budget, true)?;
    check_cancel()?;
    run_encoder(
        width_px,
        height_px,
        quality,
        budget,
        check_cancel,
        |picture| {
            let stride = stride_for(width_px, 4)?;
            // SAFETY: `image` outlives this synchronous import, which copies the
            // rows before returning; the stride fits `c_int` by the check above.
            let imported = unsafe {
                libwebp_sys::WebPPictureImportRGBA(picture, image.as_raw().as_ptr(), stride)
            };
            if imported == 0 {
                return Err(RasterError::EncodeFailed);
            }
            Ok(())
        },
    )
}

/// Encodes caller-guaranteed-opaque RGB pixels as lossy WebP; alpha is
/// dropped by construction, never by the encoder.
pub(crate) fn encode_lossy_rgb(
    image: &RgbImage,
    quality: LossyQuality,
    budget: &LiveBudget,
    check_cancel: super::CancelCheck<'_>,
    retained_bytes: u64,
) -> Result<Vec<u8>, RasterError> {
    let (width_px, height_px) = image.dimensions();
    check_native_dimensions(width_px, height_px)?;
    reserve_native(width_px, height_px, retained_bytes, budget, false)?;
    check_cancel()?;
    run_encoder(
        width_px,
        height_px,
        quality,
        budget,
        check_cancel,
        |picture| {
            let stride = stride_for(width_px, 3)?;
            // SAFETY: same copy-before-return contract as the RGBA import.
            let imported = unsafe {
                libwebp_sys::WebPPictureImportRGB(picture, image.as_raw().as_ptr(), stride)
            };
            if imported == 0 {
                return Err(RasterError::EncodeFailed);
            }
            Ok(())
        },
    )
}

/// Refuses dimensions the native encoder cannot represent before any
/// reservation or allocation.
fn check_native_dimensions(width_px: u32, height_px: u32) -> Result<(), RasterError> {
    if width_px == 0 || height_px == 0 {
        return Err(RasterError::EncodeFailed);
    }
    if width_px > MAX_NATIVE_DIMENSION_PX || height_px > MAX_NATIVE_DIMENSION_PX {
        return Err(RasterError::EncodeFailed);
    }
    Ok(())
}

/// Checks the source-derived heap bound before import. Caller images and
/// possible old/new Rust output buffers remain counted throughout the
/// synchronous call; this is admission accounting, not an allocator cap.
fn reserve_native(
    width_px: u32,
    height_px: u32,
    retained_bytes: u64,
    budget: &LiveBudget,
    has_alpha: bool,
) -> Result<(), RasterError> {
    let native =
        estimate_native_peak_bytes(width_px, height_px, has_alpha, budget.max_output_bytes());
    budget.check_retained(retained_bytes, native)
}

/// Validates the row stride the import needs; the per-axis dimension check
/// above already keeps this far below the `c_int` ceiling.
fn stride_for(width_px: u32, channels: u32) -> Result<c_int, RasterError> {
    width_px
        .checked_mul(channels)
        .and_then(|stride| c_int::try_from(stride).ok())
        .ok_or(RasterError::EncodeFailed)
}

/// Runs one synchronous encode with the pinned fields: default preset at
/// method 4, single thread, low memory, lossy color, raw exact alpha. The
/// budget reservation from the caller is held until this returns; nothing is
/// spawned, so nothing outlives the return.
fn run_encoder<'a>(
    width_px: u32,
    height_px: u32,
    quality: LossyQuality,
    budget: &LiveBudget,
    check_cancel: super::CancelCheck<'a>,
    import: impl FnOnce(*mut libwebp_sys::WebPPicture) -> Result<(), RasterError>,
) -> Result<Vec<u8>, RasterError> {
    let mut state: CallbackState<'a> = CallbackState {
        bytes: Vec::new(),
        limit_bytes: budget.max_output_bytes(),
        check_cancel: check_cancel as *const CancelCallback<'a>,
        cancelled: false,
        over_limit: false,
        allocation_failed: false,
    };
    // SAFETY: zeroing is the documented pre-state for `WebPPictureInit`;
    // every field is overwritten by init or below before any other use.
    let mut picture: libwebp_sys::WebPPicture = unsafe { std::mem::zeroed() };
    let guard = PictureGuard::new(&mut picture)?;
    // SAFETY: zeroing is the documented pre-state for `WebPConfigInit`,
    // which validates the ABI and fills every default before use.
    let mut config: libwebp_sys::WebPConfig = unsafe { std::mem::zeroed() };
    if unsafe { libwebp_sys::WebPConfigInit(&mut config) } == 0 {
        return Err(RasterError::EncodeFailed);
    }
    config.lossless = 0;
    config.quality = f32::from(quality.value());
    config.method = 4;
    config.thread_level = 0;
    config.low_memory = 1;
    config.alpha_compression = 0;
    config.alpha_quality = 100;
    config.partitions = 0;
    config.autofilter = 0;
    config.pass = 1;
    config.target_size = 0;
    config.target_PSNR = 0.0;
    config.preprocessing = 0;
    config.use_sharp_yuv = 0;
    // SAFETY: `config` is fully initialized; validation only reads it.
    if unsafe { libwebp_sys::WebPValidateConfig(&config) } == 0 {
        return Err(RasterError::EncodeFailed);
    }
    // SAFETY: the guard owns a live initialized picture; dimensions fit
    // `c_int` by the per-axis check above and are set before the import,
    // which copies exactly `height` rows of `width` pixels.
    unsafe {
        (*guard.picture()).width = width_px as c_int;
        (*guard.picture()).height = height_px as c_int;
    }
    import(guard.picture())?;
    check_cancel()?;
    let state_ptr = &mut state as *mut CallbackState<'a>;
    // SAFETY: the picture is initialized and owns its imported planes; both
    // slots point at the live caller-stack state, which outlives this
    // synchronous call.
    unsafe {
        (*guard.picture()).writer = Some(write_chunk);
        (*guard.picture()).custom_ptr = state_ptr.cast::<c_void>();
        (*guard.picture()).progress_hook = Some(report_progress);
        (*guard.picture()).user_data = state_ptr.cast::<c_void>();
    }
    // SAFETY: config and picture are initialized and validated, the import
    // owns its planes, and the state outlives this synchronous call, which
    // returns only after the last callback. The reservation above is held
    // across it; no worker permit is released early and no task is spawned.
    let encoded = unsafe { libwebp_sys::WebPEncode(&config, guard.picture()) };
    if encoded == 0 {
        if state.cancelled {
            return Err(RasterError::Cancelled);
        }
        if state.over_limit {
            return Err(RasterError::OutputTooLarge {
                len_bytes: budget.max_output_bytes().saturating_add(1),
            });
        }
        return Err(RasterError::EncodeFailed);
    }
    if state.allocation_failed {
        return Err(RasterError::EncodeFailed);
    }
    check_cancel()?;
    budget.check_output_len(state.bytes.len() as u64)?;
    if (state.bytes.capacity() as u64) > budget.max_output_bytes().saturating_add(1) {
        return Err(RasterError::OutputTooLarge {
            len_bytes: state.bytes.capacity() as u64,
        });
    }
    Ok(state.bytes)
}

/// Owns the imported and encoded native planes; freeing runs on every exit
/// path, including cancellation and writer-limit failures.
struct PictureGuard {
    picture: *mut libwebp_sys::WebPPicture,
}

impl PictureGuard {
    fn new(picture: *mut libwebp_sys::WebPPicture) -> Result<Self, RasterError> {
        // SAFETY: the caller lends a valid picture allocation for the
        // guard's lifetime; init only writes through it after its ABI check.
        if unsafe { libwebp_sys::WebPPictureInit(picture) } == 0 {
            return Err(RasterError::EncodeFailed);
        }
        Ok(Self { picture })
    }

    fn picture(&self) -> *mut libwebp_sys::WebPPicture {
        self.picture
    }
}

impl Drop for PictureGuard {
    fn drop(&mut self) {
        // SAFETY: the pointer came from a live stack picture initialized
        // above; free releases only the imported and encoded planes and
        // leaves the struct itself to its owner. It never panics.
        unsafe {
            libwebp_sys::WebPPictureFree(self.picture);
        }
    }
}

/// Native byte-emission hook: appends bounded output, then reports failure
/// so the encoder stops. A panic can never cross here; it is contained and
/// reported as a failed write.
extern "C" fn write_chunk(
    data: *const u8,
    data_size: usize,
    picture: *const libwebp_sys::WebPPicture,
) -> c_int {
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        write_chunk_inner(data, data_size, picture)
    }));
    // A contained panic reports a failed write (0), so the encoder stops.
    outcome.unwrap_or(0)
}

fn write_chunk_inner(
    data: *const u8,
    data_size: usize,
    picture: *const libwebp_sys::WebPPicture,
) -> c_int {
    if picture.is_null() {
        return 0;
    }
    // SAFETY: libwebp passes the picture it is encoding, whose `custom_ptr`
    // was set to the live caller-stack state above. The borrow ends before
    // this call returns, so shortening to a local lifetime is sound.
    let state: &mut CallbackState<'_> = unsafe {
        match ((*picture).custom_ptr as *mut CallbackState<'_>).as_mut() {
            Some(state) => state,
            None => return 0,
        }
    };
    if state.cancelled || state.over_limit || state.allocation_failed {
        return 0;
    }
    // SAFETY: the state was built from a live cancellation check that
    // outlives the synchronous encode; the borrow ends before return.
    let check: &CancelCallback<'_> = unsafe { &*state.check_cancel };
    if check().is_err() {
        state.cancelled = true;
        return 0;
    }
    if data_size == 0 {
        return 1;
    }
    if data.is_null() {
        return 0;
    }
    let end = (state.bytes.len() as u64).saturating_add(data_size as u64);
    if end > state.limit_bytes {
        state.over_limit = true;
        return 0;
    }
    // Vec growth can otherwise retain capacity above the logical limit, so
    // request exactly the incoming chunk. Ordinary OOM aborts rather than
    // unwinding, so request failure stays a typed encode failure.
    if state.bytes.try_reserve_exact(data_size).is_err() {
        state.allocation_failed = true;
        return 0;
    }
    // SAFETY: the bound above caps the copy; libwebp lends exactly
    // `data_size` readable bytes for the duration of this call.
    let chunk = unsafe { std::slice::from_raw_parts(data, data_size) };
    state.bytes.extend_from_slice(chunk);
    1
}

/// Native progress hook: polls the injected cancellation check and aborts
/// the encode when it fires. Panics are contained like the writer's.
extern "C" fn report_progress(_percent: c_int, picture: *const libwebp_sys::WebPPicture) -> c_int {
    // A contained panic reports an abort (0), so the encoder stops.
    std::panic::catch_unwind(AssertUnwindSafe(|| report_progress_inner(picture))).unwrap_or(0)
}

fn report_progress_inner(picture: *const libwebp_sys::WebPPicture) -> c_int {
    if picture.is_null() {
        return 0;
    }
    // SAFETY: same picture and state contract as the writer; `user_data`
    // was set to the live caller-stack state above.
    let state: &mut CallbackState<'_> = unsafe {
        match ((*picture).user_data as *mut CallbackState<'_>).as_mut() {
            Some(state) => state,
            None => return 0,
        }
    };
    if state.cancelled || state.over_limit || state.allocation_failed {
        return 0;
    }
    // SAFETY: same live-check contract as the writer.
    let check: &CancelCallback<'_> = unsafe { &*state.check_cancel };
    if check().is_err() {
        state.cancelled = true;
        return 0;
    }
    1
}

#[cfg(test)]
mod webp_lossy_tests;
