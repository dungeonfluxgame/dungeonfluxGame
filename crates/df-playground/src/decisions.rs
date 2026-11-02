//! The small decisions the page makes when the host's answers arrive, kept apart from the page so
//! native tests cover them: what a new set of controls changes, whether the scenario must be read
//! again and whether the scene has changed.
//!
//! They were inline in the browser-only code, which is what the fixes for stale answers, restarts,
//! transitions and animations changed, so a regression there would not have been caught.

/// What taking a new set of controls changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ControlsEffect {
    /// The scenario to show is another file (or the first), so it must be read.
    pub fixture_changed: bool,
}

/// What taking an answer's controls changes. `previous` is the scenario the page had chosen (`None`
/// before the first answer is taken), `incoming` the one the answer chooses, `first` says no answer
/// had been taken, and `new_run` that the answer is from another run of the host, whose scenario
/// file may have changed under the same name.
pub fn controls_effect(
    previous: Option<&str>,
    incoming: Option<&str>,
    first: bool,
    new_run: bool,
) -> ControlsEffect {
    ControlsEffect {
        fixture_changed: previous != incoming || first || new_run,
    }
}

/// Whether the scenario must be read now: an answer chose another one, or one is chosen and none
/// has been loaded (for instance after a failed read cleared it).
pub fn fixture_load_needed(fixture_changed: bool, loaded: bool, chosen: bool) -> bool {
    fixture_changed || (!loaded && chosen)
}

/// Whether reading `next` after `previous` is a change of scene, which starts the slides and the
/// camera from nothing. The first scenario, and the same one read again, are not.
pub fn scene_changed(previous: Option<&str>, next: &str) -> bool {
    previous.is_some_and(|previous| previous != next)
}

/// Whether the style of an element says it is animating, which is what a leaving copy must freeze.
pub fn is_animating(style: &str) -> bool {
    style.contains("animation")
}
