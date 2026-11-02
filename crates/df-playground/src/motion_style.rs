//! The inline styles for a card that slides and a camera that moves, and the decisions about when
//! the page must write them.
//!
//! Every style here is made from numbers only, so no text from a scene file reaches a style. None
//! of it touches the page, so native tests cover it; the views apply what it returns.

use crate::camera::Framing;
use crate::continuity::CameraPlan;
use crate::fixture::Slide;
use crate::limits::CAMERA_GLIDE_MS;

/// The inline style that makes a card slide: its place at the end, and an animation that starts
/// part-way through when the slide is already `elapsed_ms` under way.
pub fn slide_style(slide: &Slide, elapsed_ms: f64) -> String {
    if elapsed_ms >= f64::from(slide.duration_ms) {
        return format!("left: {}%;", slide.to_percent);
    }
    format!(
        "left: {to}%; --slide-from: {from}%; --slide-to: {to}%; animation: card-slide {duration}ms linear forwards; animation-delay: -{elapsed:.0}ms;",
        to = slide.to_percent,
        from = slide.from_percent,
        duration = slide.duration_ms,
        elapsed = elapsed_ms.max(0.0),
    )
}

/// The style that frames the world layer at rest: the framing, reached with a short glide.
pub fn resting_camera_style(to: &Framing) -> String {
    format!(
        "transform: {}; transition: transform {CAMERA_GLIDE_MS}ms linear;",
        to.transform()
    )
}

/// The whole style of a camera plan: at rest, or a move that runs from where it began to where it
/// ends, part-way through when the move is already under way.
pub fn camera_style(plan: &CameraPlan) -> String {
    if !plan.moving {
        return resting_camera_style(&plan.to);
    }
    format!(
        "transform: {to}; --cam-from: {from}; --cam-to: {to}; animation: camera-move {duration}ms linear forwards; animation-delay: -{elapsed:.0}ms;",
        from = plan.from.transform(),
        to = plan.to.transform(),
        duration = plan.duration_ms,
        elapsed = plan.elapsed_ms.max(0.0),
    )
}

/// What the page last wrote for the camera, to tell what the next draw must change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraDrawn {
    pub serial: u64,
    pub moving: bool,
    pub to: Framing,
}

impl CameraDrawn {
    pub fn of(plan: &CameraPlan) -> Self {
        Self {
            serial: plan.serial,
            moving: plan.moving,
            to: plan.to,
        }
    }
}

/// What a draw has to do to the world layer's style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraAction {
    /// Nothing: it is already drawn this way.
    Unchanged,
    /// Aim the running move at its new end and leave its clock alone, so a travelling target is
    /// followed without restarting the move.
    Retarget,
    /// Write the whole style.
    Restyle,
}

pub fn camera_action(drawn: Option<&CameraDrawn>, plan: &CameraPlan) -> CameraAction {
    let wanted = CameraDrawn::of(plan);
    match drawn {
        Some(drawn) if *drawn == wanted => CameraAction::Unchanged,
        Some(drawn) if drawn.serial == wanted.serial && drawn.moving && wanted.moving => {
            CameraAction::Retarget
        }
        _ => CameraAction::Restyle,
    }
}

/// The style a copy of the world layer gets when it is frozen where it stands: the transform the
/// browser computed, and no animation. `computed` is what the browser says the transform is now
/// (`matrix(...)` or `none`); anything else is not trusted and gives the plain freeze.
pub fn frozen_world_style(computed: &str) -> String {
    match safe_value(computed, &['(', ')', ',', '.', '-', ' ', 'e']) {
        Some(transform) if transform.starts_with("matrix") => {
            format!("transform: {transform}; animation: none; transition: none;")
        }
        _ => "animation: none; transition: none;".to_owned(),
    }
}

/// The style a copy of the sliding card gets when it is frozen where it stands: the place the
/// browser computed (`12.5px`), and no animation.
pub fn frozen_card_style(computed_left: &str) -> String {
    match safe_value(computed_left, &['.', '-']) {
        Some(left) if left.ends_with("px") => format!("left: {left}; animation: none;"),
        _ => "animation: none;".to_owned(),
    }
}

/// The text when it is made only of letters, digits and the `allowed` marks.
fn safe_value<'a>(text: &'a str, allowed: &[char]) -> Option<&'a str> {
    let text = text.trim();
    (!text.is_empty()
        && text
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || allowed.contains(&character)))
    .then_some(text)
}
