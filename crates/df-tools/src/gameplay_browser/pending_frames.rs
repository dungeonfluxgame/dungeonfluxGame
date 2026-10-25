//! Pending paint-frame bookkeeping for the page's presentation capture. Compiled natively as
//! well (declared at the crate root) so its admission rule is tested without a browser.

/// Most paint callbacks that may wait for the browser at once. A callback that has run
/// leaves the list, so this bounds only the frames still waiting.
pub(crate) const MAXIMUM_PENDING_PAINT_FRAMES: usize = 128;

/// Makes room for one more frame in a page's pending list. Frames the browser has already
/// run are dropped first, so the capacity limits only frames that still wait to run.
// Native builds use this only from tests; the browser capture is its only production caller.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn admit_pending_frame<T>(pending: &mut Vec<T>, has_run: impl Fn(&T) -> bool) -> bool {
    pending.retain(|frame| !has_run(frame));
    pending.len() < MAXIMUM_PENDING_PAINT_FRAMES
}

#[cfg(test)]
mod tests {
    use super::{MAXIMUM_PENDING_PAINT_FRAMES, admit_pending_frame};

    #[test]
    fn frames_the_browser_has_run_do_not_fill_the_pending_capacity() {
        // (id, has_run): 200 frames over a long page session have already run.
        let mut frames: Vec<(u32, bool)> = (0..200).map(|id| (id, true)).collect();
        assert!(admit_pending_frame(&mut frames, |frame| frame.1));
        assert!(frames.is_empty());
    }

    #[test]
    fn a_full_list_of_waiting_frames_refuses_another() {
        let mut frames: Vec<(u32, bool)> = (0..MAXIMUM_PENDING_PAINT_FRAMES as u32)
            .map(|id| (id, false))
            .collect();
        assert!(!admit_pending_frame(&mut frames, |frame| frame.1));
        assert_eq!(frames.len(), MAXIMUM_PENDING_PAINT_FRAMES);
    }

    #[test]
    fn only_the_waiting_frames_count_against_the_capacity() {
        let mut frames: Vec<(u32, bool)> = (0..MAXIMUM_PENDING_PAINT_FRAMES as u32)
            .map(|id| (id, id % 2 == 0))
            .collect();
        assert!(admit_pending_frame(&mut frames, |frame| frame.1));
        assert_eq!(frames.len(), MAXIMUM_PENDING_PAINT_FRAMES / 2);
    }
}
