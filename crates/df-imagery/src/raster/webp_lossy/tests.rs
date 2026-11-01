use super::*;
use std::cell::Cell;

fn test_budget(max_decoded_bytes: u64, max_output_bytes: u64) -> LiveBudget {
    LiveBudget::new(&crate::TransformLimits {
        enabled: true,
        max_concurrency: 2,
        max_decoded_bytes,
        max_output_bytes,
        queue_capacity: 64,
        deadline_secs: 30,
    })
    .unwrap()
}

#[test]
fn progress_hook_observes_borrowed_cell_cancellation() {
    let cancelled = Cell::new(false);
    let check = || {
        if cancelled.get() {
            return Err(RasterError::Cancelled);
        }
        Ok(())
    };
    let mut state: CallbackState<'_> = CallbackState {
        bytes: Vec::new(),
        limit_bytes: 1_024,
        check_cancel: &check as *const CancelCallback<'_>,
        cancelled: false,
        over_limit: false,
        allocation_failed: false,
    };
    let mut picture: libwebp_sys::WebPPicture = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libwebp_sys::WebPPictureInit(&mut picture) }, 1);
    let guard = PictureGuard::new(&mut picture).unwrap();
    unsafe {
        (*guard.picture()).user_data = (&mut state as *mut CallbackState<'_>).cast::<c_void>();
    }
    assert_eq!(report_progress_inner(guard.picture()), 1);
    cancelled.set(true);
    assert_eq!(report_progress_inner(guard.picture()), 0);
    assert!(state.cancelled);
}

#[test]
fn writer_hook_observes_borrowed_cell_cancellation() {
    let cancelled = Cell::new(false);
    let check = || {
        if cancelled.get() {
            return Err(RasterError::Cancelled);
        }
        Ok(())
    };
    let mut state: CallbackState<'_> = CallbackState {
        bytes: Vec::new(),
        limit_bytes: 1_024,
        check_cancel: &check as *const CancelCallback<'_>,
        cancelled: false,
        over_limit: false,
        allocation_failed: false,
    };
    let mut picture: libwebp_sys::WebPPicture = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libwebp_sys::WebPPictureInit(&mut picture) }, 1);
    let guard = PictureGuard::new(&mut picture).unwrap();
    unsafe {
        (*guard.picture()).custom_ptr = (&mut state as *mut CallbackState<'_>).cast::<c_void>();
    }
    let chunk = [7_u8, 8_u8];
    assert_eq!(
        write_chunk_inner(chunk.as_ptr(), chunk.len(), guard.picture()),
        1
    );
    assert_eq!(state.bytes, vec![7, 8]);
    cancelled.set(true);
    assert_eq!(
        write_chunk_inner(chunk.as_ptr(), chunk.len(), guard.picture()),
        0
    );
    assert!(state.cancelled);
}

#[test]
fn writer_hook_contains_a_panicking_check() {
    let check = || -> Result<(), RasterError> {
        panic!("test panic stays inside the hook");
    };
    let mut state: CallbackState<'_> = CallbackState {
        bytes: Vec::new(),
        limit_bytes: 1_024,
        check_cancel: &check as *const CancelCallback<'_>,
        cancelled: false,
        over_limit: false,
        allocation_failed: false,
    };
    let mut picture: libwebp_sys::WebPPicture = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libwebp_sys::WebPPictureInit(&mut picture) }, 1);
    let guard = PictureGuard::new(&mut picture).unwrap();
    unsafe {
        (*guard.picture()).custom_ptr = (&mut state as *mut CallbackState<'_>).cast::<c_void>();
        (*guard.picture()).user_data = (&mut state as *mut CallbackState<'_>).cast::<c_void>();
    }
    let chunk = [1_u8];
    assert_eq!(write_chunk(chunk.as_ptr(), chunk.len(), guard.picture()), 0);
    assert_eq!(report_progress(50, guard.picture()), 0);
}

#[test]
fn writer_growth_stays_exact_and_typed_at_the_limit() {
    let check = || Ok(());
    let mut state: CallbackState<'_> = CallbackState {
        bytes: Vec::new(),
        limit_bytes: 8,
        check_cancel: &check as *const CancelCallback<'_>,
        cancelled: false,
        over_limit: false,
        allocation_failed: false,
    };
    let mut picture: libwebp_sys::WebPPicture = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libwebp_sys::WebPPictureInit(&mut picture) }, 1);
    let guard = PictureGuard::new(&mut picture).unwrap();
    unsafe {
        (*guard.picture()).custom_ptr = (&mut state as *mut CallbackState<'_>).cast::<c_void>();
    }
    let first = [1_u8, 2, 3, 4, 5, 6];
    assert_eq!(
        write_chunk_inner(first.as_ptr(), first.len(), guard.picture()),
        1
    );
    assert!(state.bytes.capacity() <= 8 + 8);
    let second = [7_u8, 8, 9];
    assert_eq!(
        write_chunk_inner(second.as_ptr(), second.len(), guard.picture()),
        0
    );
    assert!(state.over_limit);
    assert_eq!(state.bytes.len(), 6);
}

#[test]
fn native_admission_counts_caller_images_before_any_callback() {
    let image = RgbaImage::from_pixel(1, 1, image::Rgba([30, 60, 90, 128]));
    let retained_bytes = 4;
    let required_bytes = estimate_native_peak_bytes(1, 1, true, 1_024) + retained_bytes;
    let budget = test_budget(required_bytes - 1, 1_024);
    let calls = Cell::new(0);
    let check = || {
        calls.set(calls.get() + 1);
        Ok(())
    };
    assert_eq!(
        encode_lossy_rgba(
            &image,
            LossyQuality::new(100).unwrap(),
            &budget,
            &check,
            retained_bytes,
        ),
        Err(RasterError::LiveBudgetExceeded {
            needed_bytes: required_bytes,
        })
    );
    assert_eq!(calls.get(), 0);
    let budget = test_budget(required_bytes, 1_024);
    assert!(
        encode_lossy_rgba(
            &image,
            LossyQuality::new(100).unwrap(),
            &budget,
            &check,
            retained_bytes,
        )
        .is_ok()
    );
}
