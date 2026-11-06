//! What the page must redraw when something changed, decided from the old and the new state alone.
//!
//! A change of the host's controls used to rebuild the whole page, which cut a scene transition
//! short, made every picture flash and took the focus out of the toolbar. The page now keeps its
//! pieces and redraws only the ones a change touches: the plan compares what each piece is made
//! from. A sound cue, music, a transition setting and a count draw nothing. It reads no page, so
//! native tests cover every row.

use std::collections::BTreeSet;
use std::rc::Rc;

use crate::choices::build;
use crate::controls::{Controls, Transition, TransitionStyle};
use crate::copy::status_blocks_input;
use crate::fixture::{ElementSource, Fixture, FixtureTitle, ImageRef, Slide};

/// A piece of a device that is redrawn on its own. The shared screen's figures and camera are not
/// listed: they move in place, frame by frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Part {
    TvBackdrop,
    TvGrid,
    TvSam,
    TvAnchors,
    TvPlace,
    TvCard,
    TvPanels,
    TvStatus,
    TvPills,
    TvCaption,
    TvParty,
    PhoneHead,
    PhoneStatus,
    PhoneImage,
    PhoneNarration,
    PhoneNote,
    PhoneChoices,
    PhoneComposer,
}

impl Part {
    /// The `data-part` marker the page puts on the piece, so it can find it again. It is a DOM
    /// marker, not a wire key: no reader parses it back.
    pub fn dom_marker(self) -> &'static str {
        match self {
            Self::TvBackdrop => "tv-backdrop",
            Self::TvGrid => "tv-grid",
            Self::TvSam => "tv-sam",
            Self::TvAnchors => "tv-anchors",
            Self::TvPlace => "tv-place",
            Self::TvCard => "tv-card",
            Self::TvPanels => "tv-panels",
            Self::TvStatus => "tv-status",
            Self::TvPills => "tv-pills",
            Self::TvCaption => "tv-caption",
            Self::TvParty => "tv-party",
            Self::PhoneHead => "phone-head",
            Self::PhoneStatus => "phone-status",
            Self::PhoneImage => "phone-image",
            Self::PhoneNarration => "phone-narration",
            Self::PhoneNote => "phone-note",
            Self::PhoneChoices => "phone-choices",
            Self::PhoneComposer => "phone-composer",
        }
    }
}

/// What the page is made from, as of one draw. Cheap to keep: the fixture is shared.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// The fixture files and their titles, for the scenario menu.
    pub names: Vec<FixtureTitle>,
    pub controls: Controls,
    /// The file the fixture was read from. Another file is another scene.
    pub loaded_name: Option<String>,
    /// The fixture as shown (the scene's live changes drawn on it), or the words that say why it
    /// could not be read.
    pub shown: Option<Rc<Result<Fixture, String>>>,
    pub connection_problem: Option<String>,
    /// The host's scene has live changes drawn over the fixture (the notes say so).
    pub live_changes: bool,
    pub audio_unlocked: bool,
}

/// What a redraw must do.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DrawPlan {
    /// Rebuild the toolbar.
    pub toolbar: bool,
    /// Rebuild both devices (the screen and the phone) from nothing.
    pub devices: bool,
    /// Rebuild the notes beside them.
    pub notes: bool,
    /// Redraw these pieces of the devices, leaving the rest.
    pub parts: BTreeSet<Part>,
    /// Frame the world layer again, and match the figures to the scenario's own elements.
    pub stage: bool,
}

impl DrawPlan {
    /// Draw everything.
    pub fn everything() -> Self {
        Self {
            toolbar: true,
            devices: true,
            notes: true,
            parts: BTreeSet::new(),
            stage: true,
        }
    }

    /// Whether nothing needs drawing.
    pub fn is_empty(&self) -> bool {
        !self.toolbar && !self.devices && !self.notes && !self.stage && self.parts.is_empty()
    }

    /// The plan to get from the page as `old` drew it (`None` before the first draw) to `new`.
    pub fn between(old: Option<&Snapshot>, new: &Snapshot) -> Self {
        let Some(old) = old else {
            return Self::everything();
        };
        let mut plan = Self {
            toolbar: toolbar_changed(old, new),
            ..Self::default()
        };
        let structural = old.controls.device != new.controls.device
            || old.loaded_name != new.loaded_name
            || kind(old) != kind(new);
        if structural {
            plan.devices = true;
            plan.notes = true;
            plan.stage = true;
            return plan;
        }
        plan.notes = old.connection_problem != new.connection_problem
            || old.live_changes != new.live_changes;
        if let (Some(old_fixture), Some(new_fixture)) = (fixture_of(old), fixture_of(new)) {
            plan.compare_fixtures((old_fixture, &old.controls), (new_fixture, &new.controls));
        } else if kind(new) == 2 && !same_error(old, new) {
            plan.notes = true;
        }
        plan
    }

    fn mark(&mut self, part: Part, differs: bool) {
        if differs {
            self.parts.insert(part);
        }
    }

    fn compare_fixtures(&mut self, old: (&Fixture, &Controls), new: (&Fixture, &Controls)) {
        let ((before, before_controls), (after, after_controls)) = (old, new);
        self.compare_tv(old, new);
        let choices_differ = self.compare_phone(old, new);
        // The camera can aim at a character of the backdrop's layout, so a new layout reframes.
        self.stage |= before.tv.elements != after.tv.elements
            || !same_camera(before.tv.camera.as_ref(), after.tv.camera.as_ref())
            || before.tv.backdrop_layout() != after.tv.backdrop_layout()
            || before_controls.zoom_limit != after_controls.zoom_limit;
        let words_differ = (&before.name, &before.description) != (&after.name, &after.description);
        // The notes list where the scenario breaks the choice rules, so they follow the choices.
        let warnings_differ =
            choices_differ && warnings(before, before_controls) != warnings(after, after_controls);
        self.notes |= words_differ || warnings_differ;
    }

    /// The pieces of the shared screen that differ.
    fn compare_tv(&mut self, old: (&Fixture, &Controls), new: (&Fixture, &Controls)) {
        let ((before, before_controls), (after, after_controls)) = (old, new);
        let status_before = before_controls.status_for(&before.tv.status);
        let status_after = after_controls.status_for(&after.tv.status);
        self.mark(
            Part::TvBackdrop,
            !same_image(before.tv.backdrop.as_ref(), after.tv.backdrop.as_ref()),
        );
        self.mark(
            Part::TvGrid,
            before_controls.show_grid != after_controls.show_grid,
        );
        self.mark(
            Part::TvSam,
            before_controls.show_sam != after_controls.show_sam
                || before.tv.backdrop_layout() != after.tv.backdrop_layout(),
        );
        self.mark(
            Part::TvPlace,
            before.tv.place != after.tv.place || before.tv.brand != after.tv.brand,
        );
        self.mark(
            Part::TvCard,
            !same_image(before.tv.card.as_ref(), after.tv.card.as_ref()),
        );
        self.mark(
            Part::TvPanels,
            crate::overlay::changes::panels_differ(&before.tv, &after.tv),
        );
        self.mark(
            Part::TvAnchors,
            crate::overlay::changes::anchors_differ(&before.tv, &after.tv),
        );
        self.mark(
            Part::TvPills,
            crate::overlay::changes::pills_differ(&before.tv, &after.tv),
        );
        self.mark(Part::TvStatus, status_before != status_after);
        self.mark(
            Part::TvCaption,
            before.tv.caption != after.tv.caption
                || before.tv.narration != after.tv.narration
                || before_controls.pace != after_controls.pace,
        );
        self.mark(
            Part::TvParty,
            before.tv.party != after.tv.party || before.tv.seats != after.tv.seats,
        );
    }

    /// The pieces of a phone that differ. Returns whether the choices differ.
    fn compare_phone(&mut self, old: (&Fixture, &Controls), new: (&Fixture, &Controls)) -> bool {
        let ((before, before_controls), (after, after_controls)) = (old, new);
        let status_before = before_controls.status_for(&before.phone.status);
        let status_after = after_controls.status_for(&after.phone.status);
        self.mark(
            Part::PhoneHead,
            (&before.phone.hero, &before.phone.goal) != (&after.phone.hero, &after.phone.goal),
        );
        self.mark(Part::PhoneStatus, status_before != status_after);
        self.mark(
            Part::PhoneImage,
            !same_image(before.phone.image.as_ref(), after.phone.image.as_ref()),
        );
        self.mark(
            Part::PhoneNarration,
            before.phone.narration != after.phone.narration,
        );
        self.mark(Part::PhoneNote, before.phone.note != after.phone.note);
        let choices_differ = (
            &before.phone.suggested,
            &before.phone.groups,
            &before.phone.unavailable,
            before_controls.show_unavailable,
        ) != (
            &after.phone.suggested,
            &after.phone.groups,
            &after.phone.unavailable,
            after_controls.show_unavailable,
        );
        self.mark(Part::PhoneChoices, choices_differ);
        self.mark(
            Part::PhoneComposer,
            status_blocks_input(&status_before) != status_blocks_input(&status_after),
        );
        choices_differ
    }
}

fn warnings(fixture: &Fixture, controls: &Controls) -> Vec<String> {
    build(&fixture.phone, controls.show_unavailable)
        .warnings
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn toolbar_changed(old: &Snapshot, new: &Snapshot) -> bool {
    let (before, after) = (&old.controls, &new.controls);
    old.names != new.names
        || old.audio_unlocked != new.audio_unlocked
        || (
            &before.fixture,
            before.device,
            before.pace,
            &before.status_key,
            before.show_unavailable,
            before.show_grid,
            before.show_sam,
        ) != (
            &after.fixture,
            after.device,
            after.pace,
            &after.status_key,
            after.show_unavailable,
            after.show_grid,
            after.show_sam,
        )
}

/// None, a fixture or the words that say why there is none.
fn kind(snapshot: &Snapshot) -> u8 {
    match snapshot.shown.as_deref() {
        None => 0,
        Some(Ok(_)) => 1,
        Some(Err(_)) => 2,
    }
}

fn fixture_of(snapshot: &Snapshot) -> Option<&Fixture> {
    match snapshot.shown.as_deref() {
        Some(Ok(fixture)) => Some(fixture),
        _ => None,
    }
}

fn same_error(old: &Snapshot, new: &Snapshot) -> bool {
    match (old.shown.as_deref(), new.shown.as_deref()) {
        (Some(Err(before)), Some(Err(after))) => before == after,
        _ => false,
    }
}

/// Whether two slides are the same slide, whatever the host said about how far along it is.
fn same_slide(before: Option<&Slide>, after: Option<&Slide>) -> bool {
    match (before, after) {
        (None, None) => true,
        (Some(before), Some(after)) => {
            (
                before.from_percent,
                before.to_percent,
                before.duration_ms,
                before.serial,
            ) == (
                after.from_percent,
                after.to_percent,
                after.duration_ms,
                after.serial,
            )
        }
        _ => false,
    }
}

/// Whether two pictures are the same picture, in the same place, turn and slide.
fn same_image(before: Option<&ImageRef>, after: Option<&ImageRef>) -> bool {
    match (before, after) {
        (None, None) => true,
        (Some(before), Some(after)) => {
            before.media == after.media
                && before.caption == after.caption
                && before.rotation_degrees == after.rotation_degrees
                && same_slide(before.slide.as_ref(), after.slide.as_ref())
        }
        _ => false,
    }
}

fn same_camera(
    before: Option<&crate::fixture::Camera>,
    after: Option<&crate::fixture::Camera>,
) -> bool {
    match (before, after) {
        (None, None) => true,
        (Some(before), Some(after)) => {
            (
                &before.target,
                before.zoom_percent,
                before.duration_ms,
                before.serial,
            ) == (
                &after.target,
                after.zoom_percent,
                after.duration_ms,
                after.serial,
            )
        }
        _ => false,
    }
}

/// What the page does with an answer from the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameAction {
    /// Compare with what was drawn and redraw what differs (which may be nothing).
    pub redraw: bool,
    /// Match the figures to the newest canvas and frame the camera again, without a redraw.
    pub stage: bool,
}

/// What an answer from the host asks of the page: `was_down` says the page had been showing a
/// connection problem, `controls_changed` that the controls or the scene are new, `reloaded` that
/// the scenario was read again, `canvas_changed` that the canvas is new and `live_on_tv` that the
/// scenario draws the live canvas on a shared screen that is shown. A frame that changed only the
/// canvas keeps the scene's camera: it moves the figures and frames them again, nothing else.
pub fn frame_action(
    was_down: bool,
    controls_changed: bool,
    reloaded: bool,
    canvas_changed: bool,
    live_on_tv: bool,
) -> FrameAction {
    FrameAction {
        redraw: was_down || controls_changed || reloaded,
        stage: canvas_changed && live_on_tv,
    }
}

/// Whether a scenario draws the live canvas.
pub fn wants_live(shown: Option<&Result<Fixture, String>>) -> bool {
    matches!(
        shown,
        Some(Ok(fixture)) if matches!(fixture.tv.elements, ElementSource::Live(_))
    )
}

/// The transition to draw when the scenario changes from `previous` (`None` before the first one
/// is loaded) to `next`: none for the first scenario, for the same scenario read again, for a cut
/// or for one that takes no time.
pub fn transition_for(
    previous: Option<&str>,
    next: &str,
    transition: Transition,
) -> Option<Transition> {
    let changed = previous.is_some_and(|previous| previous != next);
    (changed && transition.style != TransitionStyle::Cut && transition.duration_ms > 0)
        .then_some(transition)
}

/// The class list of the copy of the old scene that leaves: it lies over the new scene and an
/// animation of this style takes it away.
pub fn leaving_class(style: TransitionStyle) -> &'static str {
    match style {
        TransitionStyle::Wipe => "devices scene-leaving scene-leaving-wipe",
        TransitionStyle::Fade | TransitionStyle::Cut => "devices scene-leaving scene-leaving-fade",
    }
}

/// Whether an element with this class list is a leaving copy, which is taken away when its
/// animation ends.
pub fn is_leaving(class: &str) -> bool {
    class.split_whitespace().any(|name| name == "scene-leaving")
}
