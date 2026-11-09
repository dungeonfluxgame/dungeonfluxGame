//! The crossfade of the plate: when a new plate replaces the one on screen, the old
//! one stays under it and the new one fades in over it, once its image has loaded.
//!
//! This is the state machine only (what is loading, what is fading, which event belongs to it) and
//! the length of a fade from the controls' transition. `view::plate_fade` holds the layers.

use crate::controls::{Transition, TransitionStyle};

/// Where the incoming picture is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeStage {
    /// No picture is coming in.
    Idle,
    /// The picture is on the page, invisible, waiting for its image to load.
    Loading,
    /// The picture has loaded and is fading in.
    Fading,
}

/// The picture coming in and its stage. An event names the picture it is about, so one that
/// belongs to a picture already replaced is ignored.
#[derive(Debug, Default)]
pub struct PlateFade {
    incoming: Option<(String, FadeStage)>,
}

impl PlateFade {
    pub fn stage(&self) -> FadeStage {
        self.incoming
            .as_ref()
            .map_or(FadeStage::Idle, |(_, stage)| *stage)
    }

    /// The media name of the picture coming in.
    pub fn incoming(&self) -> Option<&str> {
        self.incoming.as_ref().map(|(media, _)| media.as_str())
    }

    /// A new picture `media` starts coming in. Returns the stage of the picture it replaces: one
    /// still loading was never seen and is dropped, one fading is held at the opacity it reached.
    pub fn begin(&mut self, media: &str) -> FadeStage {
        let prior = self.stage();
        self.incoming = Some((media.to_owned(), FadeStage::Loading));
        prior
    }

    /// The image of `media` loaded. True when this starts its fade.
    pub fn loaded(&mut self, media: &str) -> bool {
        match &mut self.incoming {
            Some((incoming, stage)) if incoming == media && *stage == FadeStage::Loading => {
                *stage = FadeStage::Fading;
                true
            }
            _ => false,
        }
    }

    /// The fade of `media` ended, or its image failed while loading: the picture is on screen.
    /// True when `media` was the one coming in.
    pub fn finished(&mut self, media: &str) -> bool {
        if self.incoming().is_some_and(|incoming| incoming == media) {
            self.incoming = None;
            return true;
        }
        false
    }

    /// Lets go of the picture coming in (the layers were replaced by a plain swap or a rebuild).
    /// Returns the stage it was in.
    pub fn abandon(&mut self) -> FadeStage {
        let stage = self.stage();
        self.incoming = None;
        stage
    }
}

/// How long a plate change fades, or `None` for a plain swap: a cut, or a transition that
/// takes no time.
pub fn crossfade_ms(transition: Transition) -> Option<u64> {
    (transition.style != TransitionStyle::Cut && transition.duration_ms > 0)
        .then_some(transition.duration_ms)
}
