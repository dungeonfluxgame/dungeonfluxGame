//! HOME-01: the pure A/B swap decisions of the home loop, with no browser types, so the
//! wrap can be checked without a page. The incoming clip fades in on top of the outgoing
//! one; the outgoing one stays fully opaque until the incoming one reports full opacity
//! (it has been fading for the whole fade), and only then rests. There is no moment in a
//! wrap where the two clips' combined coverage is below one clip's full opacity, so the
//! still underneath never shows through mid-fade.

/// The next clip starts this many seconds before the current one ends.
pub(crate) const SWAP_BEFORE_END: f64 = 0.5;
/// How long the incoming clip fades in, matching the stylesheet's 500 ms opacity
/// transition. The incoming clip reports full opacity once it has run this long.
pub(crate) const FADE_SECONDS: f64 = 0.5;

/// What one step of the swap decided.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SwapStep {
    /// Nothing changes this frame.
    Idle,
    /// Start this clip from its beginning, fading in on top of the outgoing one.
    StartIncoming(usize),
    /// The incoming clip is fully opaque; this outgoing clip may rest now.
    RestOutgoing(usize),
}

/// Which of the two clips is the picture, which is fading in over it, and which are told
/// to be opaque. `on` mirrors the clips' `data-on` attributes; `top` is the clip stacked
/// above the other, mirroring `data-top`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Swap {
    front: usize,
    incoming: Option<usize>,
    on: [bool; 2],
    top: usize,
}

impl Swap {
    pub(crate) fn new() -> Self {
        Self {
            front: 0,
            incoming: None,
            on: [false; 2],
            top: 0,
        }
    }

    /// The clip that is the picture.
    pub(crate) fn front(&self) -> usize {
        self.front
    }

    /// The clip fading in over the front one, while a wrap is in flight.
    pub(crate) fn incoming(&self) -> Option<usize> {
        self.incoming
    }

    /// The clip stacked on top of the other.
    pub(crate) fn top(&self) -> usize {
        self.top
    }

    /// Whether this clip is told to be opaque.
    pub(crate) fn is_on(&self, index: usize) -> bool {
        self.on[index]
    }

    /// The first clip has faded in over the still and is the picture.
    pub(crate) fn mark_shown(&mut self, index: usize) {
        self.front = index;
        self.on[index] = true;
        self.top = index;
    }

    /// Near the front clip's end, start the other clip over it. The outgoing clip stays
    /// on at full opacity; the incoming one is stacked on top and fades in.
    pub(crate) fn begin(&mut self, current: f64, duration: f64, paused: bool) -> SwapStep {
        if self.incoming.is_some() || paused || !duration.is_finite() {
            return SwapStep::Idle;
        }
        if current < duration - SWAP_BEFORE_END {
            return SwapStep::Idle;
        }
        let incoming = 1 - self.front;
        self.incoming = Some(incoming);
        self.on[incoming] = true;
        self.top = incoming;
        SwapStep::StartIncoming(incoming)
    }

    /// Once the incoming clip reports full opacity, rest the outgoing one and hand the
    /// picture over. Before that moment the outgoing clip stays on.
    pub(crate) fn settle(&mut self, incoming_current: f64) -> SwapStep {
        let Some(incoming) = self.incoming else {
            return SwapStep::Idle;
        };
        if incoming_current < FADE_SECONDS {
            return SwapStep::Idle;
        }
        let outgoing = self.front;
        self.on[outgoing] = false;
        self.front = incoming;
        self.incoming = None;
        self.top = incoming;
        SwapStep::RestOutgoing(outgoing)
    }

    /// Both clips go off and the first is the front again, ready for a new loop.
    pub(crate) fn release(&mut self) {
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::{FADE_SECONDS, Swap, SwapStep};

    #[test]
    fn the_outgoing_clip_stays_on_until_the_incoming_one_reports_full_opacity() {
        // Arrange: the first clip is the picture, near its end.
        let mut swap = Swap::new();
        swap.mark_shown(0);
        // Act: the wrap begins, then the incoming clip runs through its fade.
        assert_eq!(swap.begin(4.6, 5.08, false), SwapStep::StartIncoming(1));
        for step in 0..10 {
            let incoming_current = f64::from(step) * 0.05;
            let step_result = swap.settle(incoming_current);
            if incoming_current < FADE_SECONDS {
                // Assert: mid-fade the outgoing clip is still fully opaque beneath
                // the incoming one, which is stacked on top.
                assert_eq!(step_result, SwapStep::Idle, "at {incoming_current}");
                assert!(swap.is_on(0), "outgoing off at {incoming_current}");
                assert!(swap.is_on(1), "incoming off at {incoming_current}");
                assert_eq!(swap.top(), 1);
                assert_eq!(swap.front(), 0);
            }
        }
        // Act: the incoming clip reports full opacity.
        assert_eq!(swap.settle(FADE_SECONDS), SwapStep::RestOutgoing(0));
        // Assert: only now does the outgoing clip rest, and the incoming one is
        // the picture. There was no frame with both clips off.
        assert!(!swap.is_on(0));
        assert!(swap.is_on(1));
        assert_eq!(swap.front(), 1);
        assert_eq!(swap.incoming(), None);
    }

    #[test]
    fn the_next_wrap_swaps_back_to_the_first_clip() {
        // Arrange: one full wrap has handed the picture to the second clip.
        let mut swap = Swap::new();
        swap.mark_shown(0);
        assert_eq!(swap.begin(4.6, 5.08, false), SwapStep::StartIncoming(1));
        assert_eq!(swap.settle(FADE_SECONDS), SwapStep::RestOutgoing(0));
        // Act: the second clip nears its end and wraps back.
        assert_eq!(swap.begin(4.6, 5.08, false), SwapStep::StartIncoming(0));
        assert!(swap.is_on(0) && swap.is_on(1));
        assert_eq!(swap.top(), 0);
        assert_eq!(swap.settle(FADE_SECONDS), SwapStep::RestOutgoing(1));
        // Assert.
        assert_eq!(swap.front(), 0);
        assert!(swap.is_on(0) && !swap.is_on(1));
    }

    #[test]
    fn no_wrap_starts_while_paused_or_before_the_end_approaches() {
        // Arrange.
        let mut swap = Swap::new();
        swap.mark_shown(0);
        // Act / Assert: too early, paused, and an unknown duration all stay idle,
        // and a second begin while a wrap is in flight does not restart it.
        assert_eq!(swap.begin(2.0, 5.08, false), SwapStep::Idle);
        assert_eq!(swap.begin(4.9, 5.08, true), SwapStep::Idle);
        assert_eq!(swap.begin(4.9, f64::NAN, false), SwapStep::Idle);
        assert_eq!(swap.begin(4.9, 5.08, false), SwapStep::StartIncoming(1));
        assert_eq!(swap.begin(4.9, 5.08, false), SwapStep::Idle);
        assert_eq!(swap.incoming(), Some(1));
    }

    #[test]
    fn release_turns_both_clips_off_and_restarts_from_the_first() {
        // Arrange: a wrap is in flight.
        let mut swap = Swap::new();
        swap.mark_shown(0);
        assert_eq!(swap.begin(4.6, 5.08, false), SwapStep::StartIncoming(1));
        // Act.
        swap.release();
        // Assert.
        assert!(!swap.is_on(0) && !swap.is_on(1));
        assert_eq!(swap.front(), 0);
        assert_eq!(swap.incoming(), None);
        assert_eq!(swap.settle(FADE_SECONDS), SwapStep::Idle);
    }
}
